use super::super::admin_helpers::require_admin;
use crate::{
    AppState,
    errors::AppError,
    models::sandbox_usage_inbox::Validation,
    mw::auth::{AuthMethod, AuthUser},
    services::audit_service::AuditActor,
    services::billing::sandbox_usage::{
        contract::Receipt,
        inbox::{self, InboxError, ReceiptObservation},
        inbox_authority::{self, Authority, READ_SCOPE, Registration, WRITE_SCOPE},
        inbox_shadow, shadow_buckets,
    },
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/usage-events", post(accept))
        .route("/usage-receipts/{event_id}", get(receipt))
        .route("/usage-cursors/{stream_id}", get(cursor))
}

pub(crate) async fn prepare(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<impl IntoResponse, HttpError> {
    require_admin(&state, &auth).await?;
    inbox::prepare(&state.db).await?;
    Ok(Json(json!({"schema_version": 1})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProducerRegistration {
    namespace: String,
    owner_issuer: String,
    expected_revision: i64,
    enabled: bool,
    accepting: bool,
}

pub(crate) async fn register(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(account_id): Path<String>,
    Json(request): Json<ProducerRegistration>,
) -> Result<impl IntoResponse, HttpError> {
    require_admin(&state, &auth).await?;
    let revision = inbox_authority::register(
        &state.db,
        Registration {
            account_id: &account_id,
            namespace: &request.namespace,
            owner_issuer: &request.owner_issuer,
            expected_revision: request.expected_revision,
            enabled: request.enabled,
            accepting: request.accepting,
        },
        &AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
    )
    .await?;
    Ok(Json(json!({"revision": revision})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShadowFact {
    namespace: String,
    stream_id: String,
    fact_id: String,
}

pub(crate) async fn inspect_shadow_fact(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(fact): Json<ShadowFact>,
) -> Result<impl IntoResponse, HttpError> {
    require_admin(&state, &auth).await?;
    let result = inbox_shadow::inspect(
        &state.db,
        &state.config.jwt_issuer,
        &fact.namespace,
        &fact.stream_id,
        &fact.fact_id,
    )
    .await?
    .ok_or(HttpError::NotFound)?;
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShadowBucketRequest {
    namespace: String,
    sandbox_id: String,
    source_epoch: String,
    minute: chrono::DateTime<chrono::Utc>,
}

pub(crate) async fn observe_shadow_bucket(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(request): Json<ShadowBucketRequest>,
) -> Result<impl IntoResponse, HttpError> {
    require_admin(&state, &auth).await?;
    let row = shadow_buckets::observe(
        &state.db,
        &state.config.jwt_issuer,
        shadow_buckets::Bucket {
            namespace: &request.namespace,
            sandbox_id: &request.sandbox_id,
            source_epoch: &request.source_epoch,
            minute: request.minute,
        },
    )
    .await?
    .ok_or(HttpError::NotFound)?;
    Ok(Json(json!({
        "bucket_id": row.bucket_id,
        "revision_id": row.id,
        "revision": row.revision,
        "previous_revision_id": row.previous_revision_id,
        "input_digest": row.input_digest,
        "financial_status": "shadow",
        "provisional_total": row.provisional_total.to_string(),
        "provisional_delta": row.provisional_delta.to_string(),
        "unresolved": row.input.unresolved,
        "positions": row.positions,
    })))
}

async fn authorize(
    state: &AppState,
    auth: &AuthUser,
    scope: &'static str,
) -> Result<Authority, InboxError> {
    if auth.auth_method != AuthMethod::ServiceAccount {
        return Err(InboxError::Forbidden);
    }
    Authority::load(
        &state.db,
        &auth.user_id.to_string(),
        auth.token_jti.as_deref().unwrap_or_default(),
        &auth.scope,
        scope,
    )
    .await
}

#[derive(Serialize)]
struct ReceiptResponse {
    receipt: Receipt,
    processing: Processing,
}

#[derive(Serialize)]
struct Processing {
    validation_status: Validation,
    validation_reason: Option<String>,
    owner_status: &'static str,
    financial_status: &'static str,
}

fn receipt_response(observation: ReceiptObservation) -> Json<ReceiptResponse> {
    Json(ReceiptResponse {
        receipt: observation.receipt,
        processing: Processing {
            validation_status: observation.validation,
            validation_reason: observation.validation_reason,
            owner_status: "unresolved",
            financial_status: "shadow",
        },
    })
}

async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let authority = authorize(&state, &auth, WRITE_SCOPE).await?;
    let (created, receipt) = inbox::accept(&state.db, &authority, &body).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, receipt_response(receipt)))
}

async fn receipt(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(event_id): Path<String>,
) -> Result<impl IntoResponse, HttpError> {
    let authority = authorize(&state, &auth, READ_SCOPE).await?;
    let receipt = inbox::receipt(&state.db, &authority, &event_id)
        .await?
        .ok_or(HttpError::NotFound)?;
    Ok(receipt_response(receipt))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorQuery {
    page: Option<String>,
}

async fn cursor(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(stream_id): Path<String>,
    Query(query): Query<CursorQuery>,
) -> Result<impl IntoResponse, HttpError> {
    let authority = authorize(&state, &auth, READ_SCOPE).await?;
    let page = inbox::cursor(&state.db, &authority, &stream_id, query.page.as_deref())
        .await?
        .ok_or(HttpError::NotFound)?;
    Ok(Json(page))
}

pub(crate) enum HttpError {
    NotFound,
    Inbox(InboxError),
    Application(AppError),
}

impl From<InboxError> for HttpError {
    fn from(error: InboxError) -> Self {
        Self::Inbox(error)
    }
}

impl From<AppError> for HttpError {
    fn from(error: AppError) -> Self {
        Self::Application(error)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        match self {
            Self::Application(error) => error.into_response(),
            Self::NotFound => AppError::NotFound("Usage evidence not found".into()).into_response(),
            Self::Inbox(InboxError::Invalid(reason)) => {
                AppError::ValidationError(reason.into()).into_response()
            }
            Self::Inbox(InboxError::Conflict("cursor_changed")) => {
                AppError::Conflict("Usage cursor changed; restart pagination".into())
                    .into_response()
            }
            Self::Inbox(InboxError::Conflict(reason)) => {
                AppError::Conflict(reason.into()).into_response()
            }
            Self::Inbox(InboxError::Prerequisite(reason)) => (
                StatusCode::CONFLICT,
                Json(
                    json!({"error": reason, "message": "Deliver the prerequisite before retrying"}),
                ),
            )
                .into_response(),
            Self::Inbox(InboxError::Forbidden) => {
                AppError::Forbidden("Usage producer is unauthorized".into()).into_response()
            }
            Self::Inbox(
                InboxError::Unavailable | InboxError::Corrupt | InboxError::Database(_),
            ) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "usage_custody_unavailable"})),
            )
                .into_response(),
        }
    }
}
