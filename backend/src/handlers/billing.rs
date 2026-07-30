use axum::{
    Json,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use hmac::{Hmac, Mac};
use mongodb::bson::{self, Bson, Document, doc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::{BillingRateCache, COLLECTION_NAME as BILLING_RATE_CACHE};
use crate::models::billing_topup_session::BillingTopUpStatus;
use crate::models::billing_wallet::{BillingWallet, CollectionState, PlanKind};
use crate::models::service_billing::BillingMetric;
use crate::models::usage_meter::COLLECTION_NAME as USAGE_METER;
use crate::mw::auth::AuthUser;

type HmacSha256 = Hmac<Sha256>;

const LAGO_SIGNATURE_HEADER: &str = "x-lago-signature";
const LAGO_SIGNATURE_ALGORITHM_HEADER: &str = "x-lago-signature-algorithm";
const LAGO_UNIQUE_KEY_HEADER: &str = "x-lago-unique-key";
const LAGO_HMAC_SHA256_ALGORITHM: &str = "hmac";

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    pub period: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BillingUsageResponse {
    pub owner_id: String,
    pub period: String,
    pub rows: Vec<BillingUsageRow>,
    pub totals: BillingUsageTotals,
    pub billing: BillingReadOnlyBlock,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BillingUsageRow {
    pub service_slug: Option<String>,
    pub service_id: Option<String>,
    pub metric: BillingMetric,
    pub lago_metric_code: String,
    pub layer: String,
    pub quantity: i64,
    pub requests: i64,
    pub bytes: i64,
    pub events: i64,
    pub lago_acked: bool,
    /// False for usage metered on a service without platform billing:
    /// observability only, no cost, never pushed to Lago.
    pub billable: bool,
    pub estimated_credits_micros: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BillingUsageTotals {
    pub quantity: i64,
    pub requests: i64,
    pub bytes: i64,
    pub events: i64,
    pub estimated_credits_micros: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BillingReadOnlyBlock {
    pub charging_enabled: bool,
    pub lago_configured: bool,
    pub source: String,
    pub rates_are_approximate: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProvisionWalletRequest {
    pub owner_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TopUpRequest {
    pub amount_credits: i64,
    pub idempotency_key: String,
    pub owner_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BillingWalletResponse {
    pub owner_id: String,
    pub plan_kind: PlanKind,
    pub collection_state: CollectionState,
    pub balance_credits: i64,
    pub reserved_credits: i64,
    pub pending_lago_debits: i64,
    pub available_credits: i64,
    pub available_with_overdraft_credits: i64,
    pub has_payment_instrument: bool,
    pub overdraft_cap_credits: i64,
    pub suspended: bool,
    pub lago_customer_id: String,
    pub lago_subscription_id: Option<String>,
    pub lago_wallet_id: Option<String>,
    pub balance_synced_at: chrono::DateTime<Utc>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub created: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopUpResponse {
    pub owner_id: String,
    pub amount_credits: i64,
    pub idempotency_key: String,
    pub checkout_url: String,
    pub payment_provider: Option<String>,
    pub lago_wallet_transaction_id: Option<String>,
    pub lago_invoice_id: Option<String>,
    pub status: BillingTopUpStatus,
    pub reused: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LagoWebhookResponse {
    pub ok: bool,
    pub action: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopUpHistoryEntry {
    pub id: String,
    pub created_at: chrono::DateTime<Utc>,
    pub amount_credits: i64,
    /// paid | pending | expired | failed | voided
    pub status: String,
    pub invoice_number: Option<String>,
    pub lago_invoice_id: Option<String>,
    /// Present while the checkout can still be completed.
    pub checkout_url: Option<String>,
    /// True when a receipt PDF can be downloaded for this top-up.
    pub receipt_available: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopUpHistoryResponse {
    pub owner_id: String,
    pub topups: Vec<TopUpHistoryEntry>,
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
}

#[derive(Debug, Deserialize)]
pub struct TopUpHistoryQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceDownloadResponse {
    pub file_url: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/usage",
    tag = "Billing",
    params(
        ("period" = Option<String>, Query, description = "Usage period: 24h, 7d, 30d, 90d, or all")
    ),
    responses(
        (status = 200, description = "Billing usage summary", body = BillingUsageResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_usage(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<UsageQuery>,
) -> AppResult<Json<BillingUsageResponse>> {
    let owner_id = auth_user.user_id.to_string();
    let period = query.period.unwrap_or_else(|| "30d".to_string());
    let since = period_start(&period);
    let mut match_doc = doc! {
        "billing_owner_id": &owner_id,
        "quantity": { "$ne": null },
        // The billing page shows charged usage only: rows without a wallet
        // were metered for observability (service not platform_billable)
        // and never cost anything. Resale rows carry a wallet and stay in.
        "wallet_id": { "$ne": null },
        "created_at": { "$gte": bson::DateTime::from_chrono(since) },
        "status": { "$in": ["finalized", "dead_letter"] },
    };
    match_doc.insert(
        "$or",
        vec![
            doc! { "status": "finalized" },
            doc! { "status": "dead_letter", "forwarded": true },
        ],
    );

    let pipeline = vec![
        doc! { "$match": match_doc },
        doc! {
            "$group": {
                "_id": {
                    "service_slug": "$service_slug",
                    "service_id": "$service_id",
                    "metric": "$metric",
                    "lago_metric_code": "$lago_metric_code",
                    "layer": "$layer",
                    "lago_acked": "$lago_acked",
                    // Rows without a wallet were metered for observability
                    // only (service not platform_billable); they carry no
                    // cost and are never pushed to Lago.
                    "billable": { "$ne": [{ "$ifNull": ["$wallet_id", null] }, null] },
                },
                "quantity": { "$sum": "$quantity" },
                "events": { "$sum": 1 },
            }
        },
        doc! { "$sort": { "_id.service_slug": 1, "_id.layer": 1, "_id.metric": 1 } },
    ];

    let mut cursor = state
        .db
        .collection::<Document>(USAGE_METER)
        .aggregate(pipeline)
        .await?;
    let mut rows = Vec::new();
    while let Some(doc) = cursor.try_next().await? {
        let Some(id_doc) = doc.get_document("_id").ok() else {
            continue;
        };
        let metric = id_doc
            .get_str("metric")
            .ok()
            .and_then(parse_metric)
            .unwrap_or_default();
        let quantity = doc_i64(&doc, "quantity").unwrap_or(0);
        let lago_metric_code = id_doc.get_str("lago_metric_code").unwrap_or("").to_string();
        let billable = id_doc.get_bool("billable").unwrap_or(false);
        let estimated_credits_micros = if billable {
            find_rate(&state.db, &lago_metric_code, None)
                .await?
                .map(|rate| rate.credits_per_unit_micros.saturating_mul(quantity))
        } else {
            Some(0)
        };
        rows.push(BillingUsageRow {
            service_slug: id_doc.get_str("service_slug").ok().map(ToString::to_string),
            service_id: id_doc.get_str("service_id").ok().map(ToString::to_string),
            metric,
            lago_metric_code,
            layer: id_doc.get_str("layer").unwrap_or("platform").to_string(),
            quantity,
            requests: if metric == BillingMetric::Requests {
                quantity
            } else {
                0
            },
            bytes: if metric == BillingMetric::Bytes {
                quantity
            } else {
                0
            },
            events: doc_i64(&doc, "events").unwrap_or(0),
            lago_acked: id_doc.get_bool("lago_acked").unwrap_or(false),
            billable,
            estimated_credits_micros,
        });
    }

    let totals = BillingUsageTotals {
        quantity: rows.iter().map(|row| row.quantity).sum(),
        requests: rows.iter().map(|row| row.requests).sum(),
        bytes: rows.iter().map(|row| row.bytes).sum(),
        events: rows.iter().map(|row| row.events).sum(),
        estimated_credits_micros: sum_optional(rows.iter().map(|row| row.estimated_credits_micros)),
    };

    Ok(Json(BillingUsageResponse {
        owner_id,
        period,
        rows,
        totals,
        billing: BillingReadOnlyBlock {
            charging_enabled: state.billing.billing_enabled() && state.billing.lago_configured(),
            lago_configured: state.billing.lago_configured(),
            source: "usage_meter".to_string(),
            rates_are_approximate: true,
        },
    }))
}

/// Reject wallet surfaces for owners outside the staged billing rollout.
async fn ensure_billing_rollout(state: &AppState, owner_id: &str, actor_id: &str) -> AppResult<()> {
    if crate::services::feature_flag_service::billing_rollout_enabled(&state.db, owner_id, actor_id)
        .await?
    {
        return Ok(());
    }
    Err(AppError::Forbidden(
        "Billing is not enabled for this account".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/wallet",
    tag = "Billing",
    responses(
        (status = 200, description = "Billing wallet", body = BillingWalletResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_wallet(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<BillingWalletResponse>> {
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_wallet_management(&auth_user.user_id.to_string(), None)
        .await?;
    ensure_billing_rollout(&state, &owner.owner_id, &auth_user.user_id.to_string()).await?;
    if let Some(wallet) = state.billing.get_wallet(&owner.owner_id).await? {
        return Ok(Json(BillingWalletResponse::from_wallet(wallet, false)));
    }
    let provisioned = state.billing.ensure_wallet(&owner.owner_id).await?;

    Ok(Json(BillingWalletResponse::from_wallet(
        provisioned.wallet,
        provisioned.created,
    )))
}

#[utoipa::path(
    post,
    path = "/api/v1/billing/wallet",
    tag = "Billing",
    request_body = ProvisionWalletRequest,
    responses(
        (status = 200, description = "Provisioned billing wallet", body = BillingWalletResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn provision_wallet(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<ProvisionWalletRequest>,
) -> AppResult<Json<BillingWalletResponse>> {
    let actor_id = auth_user.user_id.to_string();
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_wallet_management(&actor_id, body.owner_id.as_deref())
        .await?;
    ensure_billing_rollout(&state, &owner.owner_id, &actor_id).await?;
    let provisioned = state.billing.ensure_wallet(&owner.owner_id).await?;

    Ok(Json(BillingWalletResponse::from_wallet(
        provisioned.wallet,
        provisioned.created,
    )))
}

#[utoipa::path(
    post,
    path = "/api/v1/billing/topup",
    tag = "Billing",
    request_body = TopUpRequest,
    responses(
        (status = 200, description = "Hosted top-up checkout session", body = TopUpResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_topup(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<TopUpRequest>,
) -> AppResult<Json<TopUpResponse>> {
    let actor_id = auth_user.user_id.to_string();
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_wallet_management(&actor_id, body.owner_id.as_deref())
        .await?;
    ensure_billing_rollout(&state, &owner.owner_id, &actor_id).await?;
    let checkout = state
        .billing
        .create_topup_checkout(&owner.owner_id, body.amount_credits, &body.idempotency_key)
        .await?;
    let checkout_url = checkout.session.payment_url.clone().ok_or_else(|| {
        AppError::BillingProviderUnavailable(
            "Billing top-up session does not have a hosted checkout URL".to_string(),
        )
    })?;

    Ok(Json(TopUpResponse {
        owner_id: checkout.session.owner_id,
        amount_credits: checkout.session.amount_credits,
        idempotency_key: checkout.session.idempotency_key,
        checkout_url,
        payment_provider: checkout.session.payment_provider,
        lago_wallet_transaction_id: checkout.session.lago_wallet_transaction_id,
        lago_invoice_id: checkout.session.lago_invoice_id,
        status: checkout.session.status,
        reused: checkout.reused,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/topups",
    tag = "Billing",
    responses(
        (status = 200, description = "Top-up history", body = TopUpHistoryResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_topups(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<TopUpHistoryQuery>,
) -> AppResult<Json<TopUpHistoryResponse>> {
    let actor_id = auth_user.user_id.to_string();
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_wallet_management(&actor_id, None)
        .await?;
    ensure_billing_rollout(&state, &owner.owner_id, &actor_id).await?;

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(10).clamp(1, 50);
    let filter = doc! { "owner_id": &owner.owner_id };
    let collection = state
        .db
        .collection::<crate::models::billing_topup_session::BillingTopUpSession>(
            crate::models::billing_topup_session::COLLECTION_NAME,
        );
    let total = collection.count_documents(filter.clone()).await?;
    let sessions: Vec<crate::models::billing_topup_session::BillingTopUpSession> = collection
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .skip(u64::from((page - 1) * per_page))
        .limit(i64::from(per_page))
        .await?
        .try_collect()
        .await?;

    // One Lago call resolves payment outcomes for every session: the
    // local store only tracks checkout creation, while paid/voided live
    // on the credit invoice. Lago being unreachable degrades to local
    // statuses rather than failing the page.
    let invoices = match state.billing.lago_client() {
        Some(lago) => lago
            .credit_invoices(&owner.owner_id)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let invoices_by_id: std::collections::HashMap<
        &str,
        &crate::services::billing::lago_client::InvoiceSummary,
    > = invoices
        .iter()
        .map(|invoice| (invoice.lago_id.as_str(), invoice))
        .collect();

    let checkout_expiry_cutoff = Utc::now() - Duration::hours(24);
    let topups = sessions
        .into_iter()
        .map(|session| {
            let invoice = session
                .lago_invoice_id
                .as_deref()
                .and_then(|id| invoices_by_id.get(id).copied());
            let status = match invoice {
                Some(invoice) if invoice.status == "voided" => "voided",
                Some(invoice) if invoice.payment_status == "succeeded" => "paid",
                Some(invoice) if invoice.payment_status == "failed" => "failed",
                _ if matches!(
                    session.status,
                    crate::models::billing_topup_session::BillingTopUpStatus::Failed
                ) =>
                {
                    "failed"
                }
                // Stripe checkout sessions expire after 24 hours and Lago
                // returns the same cached session per transaction, so an
                // old pending checkout can no longer be completed.
                _ if session.created_at < checkout_expiry_cutoff => "expired",
                _ => "pending",
            };
            TopUpHistoryEntry {
                id: session.id,
                created_at: session.created_at,
                amount_credits: session.amount_credits,
                status: status.to_string(),
                invoice_number: invoice.map(|invoice| invoice.number.clone()),
                lago_invoice_id: session.lago_invoice_id,
                checkout_url: (status == "pending")
                    .then_some(session.payment_url)
                    .flatten(),
                receipt_available: status == "paid",
            }
        })
        .collect();

    Ok(Json(TopUpHistoryResponse {
        owner_id: owner.owner_id,
        topups,
        page,
        per_page,
        total,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/invoices/{invoice_id}/download",
    tag = "Billing",
    params(("invoice_id" = String, Path, description = "Lago invoice id")),
    responses(
        (status = 200, description = "Signed receipt download URL", body = InvoiceDownloadResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn download_invoice(
    State(state): State<AppState>,
    auth_user: AuthUser,
    axum::extract::Path(invoice_id): axum::extract::Path<String>,
) -> AppResult<Json<InvoiceDownloadResponse>> {
    let actor_id = auth_user.user_id.to_string();
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_wallet_management(&actor_id, None)
        .await?;
    ensure_billing_rollout(&state, &owner.owner_id, &actor_id).await?;

    let lago = state.billing.lago_client().ok_or_else(|| {
        AppError::BillingNotConfigured("Lago client is not configured".to_string())
    })?;
    // Ownership check before touching the document: the invoice's Lago
    // customer must be the resolved billing owner. Foreign or unknown
    // invoices 404 identically so existence is not leaked.
    let invoice = lago
        .invoice_summary(&invoice_id)
        .await?
        .filter(|invoice| invoice.external_customer_id.as_deref() == Some(owner.owner_id.as_str()))
        .ok_or_else(|| AppError::NotFound("Invoice not found".to_string()))?;

    let file_url = lago
        .invoice_download_url(&invoice.lago_id)
        .await?
        .ok_or_else(|| {
            AppError::BillingProviderUnavailable(
                "The receipt is still being generated; try again shortly".to_string(),
            )
        })?;

    Ok(Json(InvoiceDownloadResponse { file_url }))
}

pub async fn lago_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<(StatusCode, Json<LagoWebhookResponse>)> {
    let expected_secret = state.config.lago_webhook_secret.as_deref().ok_or_else(|| {
        tracing::warn!("Lago webhook received but LAGO_WEBHOOK_SECRET is not configured");
        AppError::Unauthorized("Lago webhook signature verification failed".to_string())
    })?;

    verify_lago_signature(&headers, &body, expected_secret).map_err(|error| {
        tracing::warn!(error = %error, "Lago webhook signature verification failed");
        AppError::Unauthorized("Lago webhook signature verification failed".to_string())
    })?;

    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("Lago webhook payload must be valid JSON".to_string()))?;
    let event_type = lago_event_type(&payload).ok_or_else(|| {
        AppError::BadRequest("Lago webhook payload is missing webhook_type".to_string())
    })?;
    let unique_key = headers
        .get(LAGO_UNIQUE_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    let lago = state.billing.lago_client();
    let outcome = crate::services::billing::webhook::handle_lago_webhook_event(
        &state.db,
        lago.as_deref(),
        event_type,
        &payload,
    )
    .await?;

    tracing::info!(
        event_type,
        unique_key,
        action = outcome.action.as_str(),
        owner_id = outcome.owner_id.as_deref().unwrap_or(""),
        customer_id = outcome.customer_id.as_deref().unwrap_or(""),
        "Lago webhook processed"
    );

    Ok((
        StatusCode::OK,
        Json(LagoWebhookResponse {
            ok: true,
            action: outcome.action.as_str().to_string(),
        }),
    ))
}

fn verify_lago_signature(
    headers: &HeaderMap,
    body: &[u8],
    secret: &str,
) -> Result<(), &'static str> {
    if secret.trim().is_empty() {
        return Err("missing expected secret");
    }

    let algorithm =
        header_str(headers, LAGO_SIGNATURE_ALGORITHM_HEADER).unwrap_or(LAGO_HMAC_SHA256_ALGORITHM);
    if !algorithm.eq_ignore_ascii_case(LAGO_HMAC_SHA256_ALGORITHM) {
        return Err("unsupported signature algorithm");
    }

    let received = header_str(headers, LAGO_SIGNATURE_HEADER).ok_or("missing signature")?;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| "invalid expected secret")?;
    mac.update(body);
    let expected = BASE64_STANDARD.encode(mac.finalize().into_bytes());
    if constant_time_str_eq(received.trim(), &expected) {
        Ok(())
    } else {
        Err("signature mismatch")
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn constant_time_str_eq(received: &str, expected: &str) -> bool {
    let received_hash = Sha256::digest(received.as_bytes());
    let expected_hash = Sha256::digest(expected.as_bytes());
    received_hash.ct_eq(&expected_hash).into()
}

fn lago_event_type(payload: &serde_json::Value) -> Option<&str> {
    payload
        .get("webhook_type")
        .or_else(|| payload.get("event_type"))
        .or_else(|| payload.get("event"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn period_start(period: &str) -> chrono::DateTime<Utc> {
    let now = Utc::now();
    match period {
        "24h" | "1d" => now - Duration::days(1),
        "7d" => now - Duration::days(7),
        "90d" => now - Duration::days(90),
        "all" => now - Duration::days(3650),
        _ => now - Duration::days(30),
    }
}

async fn find_rate(
    db: &mongodb::Database,
    lago_metric_code: &str,
    model: Option<&str>,
) -> AppResult<Option<BillingRateCache>> {
    let specific_id = BillingRateCache::cache_id(lago_metric_code, model);
    if let Some(rate) = db
        .collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .find_one(doc! { "_id": specific_id })
        .await?
    {
        return Ok(Some(rate));
    }
    db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .find_one(doc! { "_id": BillingRateCache::cache_id(lago_metric_code, None) })
        .await
        .map_err(Into::into)
}

fn parse_metric(value: &str) -> Option<BillingMetric> {
    match value {
        "tokens" => Some(BillingMetric::Tokens),
        "requests" => Some(BillingMetric::Requests),
        "bytes" => Some(BillingMetric::Bytes),
        _ => None,
    }
}

fn doc_i64(doc: &Document, key: &str) -> Option<i64> {
    match doc.get(key)? {
        Bson::Int32(v) => Some(i64::from(*v)),
        Bson::Int64(v) => Some(*v),
        Bson::Double(v) => Some(v.round() as i64),
        _ => None,
    }
}

fn sum_optional(values: impl Iterator<Item = Option<i64>>) -> Option<i64> {
    let mut saw_value = false;
    let mut total = 0_i64;
    for value in values.flatten() {
        saw_value = true;
        total = total.saturating_add(value);
    }
    saw_value.then_some(total)
}

impl BillingWalletResponse {
    fn from_wallet(wallet: BillingWallet, created: bool) -> Self {
        let available_credits = wallet.available_credits();
        let available_with_overdraft_credits = wallet.available_with_overdraft_credits();
        let suspended = wallet.is_suspended();

        Self {
            owner_id: wallet.owner_id,
            plan_kind: wallet.plan_kind,
            collection_state: wallet.collection_state,
            balance_credits: wallet.balance_credits,
            reserved_credits: wallet.reserved_credits,
            pending_lago_debits: wallet.pending_lago_debits,
            available_credits,
            available_with_overdraft_credits,
            has_payment_instrument: wallet.has_payment_instrument,
            overdraft_cap_credits: wallet.overdraft_cap_credits,
            suspended,
            lago_customer_id: wallet.lago_customer_id,
            lago_subscription_id: wallet.lago_subscription_id,
            lago_wallet_id: wallet.lago_wallet_id,
            balance_synced_at: wallet.balance_synced_at,
            created_at: wallet.created_at,
            updated_at: wallet.updated_at,
            created,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_headers(secret: &str, body: &[u8]) -> HeaderMap {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let signature = BASE64_STANDARD.encode(mac.finalize().into_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(LAGO_SIGNATURE_HEADER, signature.parse().unwrap());
        headers.insert(
            LAGO_SIGNATURE_ALGORITHM_HEADER,
            LAGO_HMAC_SHA256_ALGORITHM.parse().unwrap(),
        );
        headers
    }

    #[test]
    fn lago_signature_accepts_valid_hmac() {
        let body = br#"{"webhook_type":"wallet.updated"}"#;
        let headers = signed_headers("secret", body);

        assert!(verify_lago_signature(&headers, body, "secret").is_ok());
    }

    #[test]
    fn lago_signature_rejects_tampered_body() {
        let headers = signed_headers("secret", br#"{"webhook_type":"wallet.updated"}"#);

        assert!(
            verify_lago_signature(
                &headers,
                br#"{"webhook_type":"subscription.started"}"#,
                "secret"
            )
            .is_err()
        );
    }

    #[test]
    fn lago_event_type_reads_webhook_type() {
        let payload = serde_json::json!({ "webhook_type": "wallet.updated" });

        assert_eq!(lago_event_type(&payload), Some("wallet.updated"));
    }
}
