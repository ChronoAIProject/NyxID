use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, Document, doc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::AppResult;
use crate::models::billing_rate_cache::{BillingRateCache, COLLECTION_NAME as BILLING_RATE_CACHE};
use crate::models::service_billing::BillingMetric;
use crate::models::usage_meter::COLLECTION_NAME as USAGE_METER;
use crate::mw::auth::AuthUser;

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

#[derive(Debug, Serialize, ToSchema)]
pub struct BillingWalletResponse {
    pub owner_id: String,
    pub charging_enabled: bool,
    pub lago_configured: bool,
    pub wallet_configured: bool,
    pub status: String,
    pub balance_credits: Option<i64>,
    pub reserved_credits: Option<i64>,
    pub pending_lago_debits: Option<i64>,
    pub available_credits: Option<i64>,
    pub source: String,
    pub invoices: Vec<BillingInvoiceSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BillingInvoiceSummary {
    pub id: String,
    pub status: String,
    pub amount_credits_micros: Option<i64>,
    pub currency: Option<String>,
    pub hosted_url: Option<String>,
    pub issued_at: Option<String>,
    pub due_at: Option<String>,
}

pub async fn get_wallet(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<BillingWalletResponse>> {
    Ok(Json(BillingWalletResponse {
        owner_id: auth_user.user_id.to_string(),
        charging_enabled: state.billing.billing_enabled(),
        lago_configured: state.billing.lago_configured(),
        wallet_configured: false,
        status: "not_configured".to_string(),
        balance_credits: None,
        reserved_credits: None,
        pending_lago_debits: None,
        available_credits: None,
        source: "usage_meter".to_string(),
        invoices: Vec::new(),
    }))
}

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
        let model_rate = find_rate(&state.db, &lago_metric_code, None).await?;
        let estimated_credits_micros =
            model_rate.map(|rate| rate.credits_per_unit_micros.saturating_mul(quantity));
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
            charging_enabled: false,
            lago_configured: state.billing.lago_configured(),
            source: "usage_meter".to_string(),
            rates_are_approximate: true,
        },
    }))
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
