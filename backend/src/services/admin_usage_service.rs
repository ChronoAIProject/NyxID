//! Platform-wide, read-only meter reporting. MongoDB owns every reduction,
//! including legacy pricing, ranking order, and pagination.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, Document, doc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::errors::{AppError, AppResult};
use crate::models::service_billing::BillingMetric;
use crate::models::usage_meter::COLLECTION_NAME;

const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
const TOKEN_FIELDS: &[&str] = &[
    "prompt_tokens",
    "completion_tokens",
    "cached_tokens",
    "cache_creation_tokens",
];
const COST_FIELDS: &[&str] = &[
    "gross_cost_micros",
    "wallet_cost_micros",
    "grant_cost_micros",
    "allowance_cost_micros",
];
const COUNT_FIELDS: &[&str] = &[
    "requests",
    "events",
    "exact_cost_events",
    "legacy_cost_events",
    "unknown_cost_events",
];

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AdminUsageQuery {
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub user: Option<String>,
    pub service: Option<String>,
    /// Ranking quantity unit; defaults to tokens. Never adds unlike units.
    pub metric: Option<String>,
    /// quantity, requests, cost, total_tokens, prompt_tokens, completion_tokens,
    /// cached_tokens, or cache_creation_tokens (descending).
    pub sort: Option<String>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UsageWindow {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub period: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, ToSchema)]
pub struct UsageStats {
    pub requests: i64,
    pub events: i64,
    pub quantities: BTreeMap<String, i64>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cache_creation_tokens: i64,
    /// Prompt + completion, following provider accounting. Cache counts may
    /// overlap prompt counts and must not be added to this total.
    pub total_tokens: i64,
    pub gross_cost_micros: Option<i64>,
    pub wallet_cost_micros: Option<i64>,
    pub grant_cost_micros: Option<i64>,
    pub allowance_cost_micros: Option<i64>,
    pub exact_cost_events: i64,
    pub legacy_cost_events: i64,
    /// Billable legacy events whose current cached rate is unavailable.
    pub unknown_cost_events: i64,
    pub unique_users: i64,
    pub unique_services: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UsageIdentity {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub user_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UsageServiceIdentity {
    pub service_id: Option<String>,
    pub service_slug: Option<String>,
    pub service_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UsageCredentialClass {
    pub credential_class: String,
    #[serde(flatten)]
    pub usage: UsageStats,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UsageService {
    #[serde(flatten)]
    pub service: UsageServiceIdentity,
    #[serde(flatten)]
    pub usage: UsageStats,
    pub by_credential_class: Vec<UsageCredentialClass>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UsageRanking {
    pub user: UsageIdentity,
    pub billing_owner: Option<UsageIdentity>,
    #[serde(flatten)]
    pub service: UsageServiceIdentity,
    #[serde(flatten)]
    pub usage: UsageStats,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUsageResponse {
    pub window: UsageWindow,
    pub totals: UsageStats,
    pub by_service: Vec<UsageService>,
    pub by_credential_class: Vec<UsageCredentialClass>,
    pub ranking: Vec<UsageRanking>,
    pub ranking_total: i64,
    pub page: u64,
    pub per_page: u64,
    pub ranking_metric: String,
    /// Window/user-scoped options, independent of the selected service.
    pub services: Vec<UsageServiceIdentity>,
    pub selected_user: Option<UsageIdentity>,
}

pub struct UsageParams {
    pub window: UsageWindow,
    pub user: Option<String>,
    pub service: Option<String>,
    pub metric: String,
    pub sort: String,
    pub page: u64,
    pub per_page: u64,
    pub offset: i64,
}

impl AdminUsageQuery {
    pub fn validate(self, now: DateTime<Utc>) -> AppResult<UsageParams> {
        let invalid = |message: &str| AppError::ValidationError(message.into());
        let window = match (self.from, self.to) {
            (Some(from), Some(to)) => {
                if self.period.is_some() {
                    return Err(invalid("Use either period or from + to"));
                }
                let parse = |value: &str| {
                    DateTime::parse_from_rfc3339(value)
                        .map(|date| date.with_timezone(&Utc))
                        .map_err(|_| invalid("from and to must be RFC 3339 timestamps"))
                };
                UsageWindow {
                    from: parse(&from)?,
                    to: parse(&to)?,
                    period: "custom".into(),
                }
            }
            (None, None) => {
                let period = self.period.unwrap_or_else(|| "24h".into());
                let days = match period.as_str() {
                    "24h" => 1,
                    "7d" => 7,
                    "30d" => 30,
                    _ => return Err(invalid("period must be 24h, 7d, or 30d")),
                };
                UsageWindow {
                    from: now - chrono::Duration::days(days),
                    to: now,
                    period,
                }
            }
            _ => return Err(invalid("from and to must be supplied together")),
        };
        if window.from >= window.to || window.to - window.from > chrono::Duration::days(31) {
            return Err(invalid("Usage window must be positive and at most 31 days"));
        }
        let user = self
            .user
            .map(|id| {
                uuid::Uuid::parse_str(&id)
                    .map(|id| id.to_string())
                    .map_err(|_| invalid("user must be a UUID"))
            })
            .transpose()?;
        let service = self.service.map(|value| value.trim().to_string());
        if service
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 256)
        {
            return Err(invalid("service must contain 1 to 256 characters"));
        }
        let metric = self.metric.unwrap_or_else(|| "tokens".into());
        if !BillingMetric::ALL
            .iter()
            .any(|value| value.as_str() == metric)
        {
            return Err(invalid("Unknown ranking metric"));
        }
        let sort = self.sort.unwrap_or_else(|| "requests".into());
        if !["quantity", "requests", "cost", "total_tokens"].contains(&sort.as_str())
            && !TOKEN_FIELDS.contains(&sort.as_str())
        {
            return Err(invalid("Unknown ranking sort"));
        }
        let page = self.page.unwrap_or(1);
        let per_page = self.per_page.unwrap_or(25);
        if page == 0 || !(1..=100).contains(&per_page) {
            return Err(invalid(
                "page must be positive and per_page between 1 and 100",
            ));
        }
        let offset = page
            .checked_sub(1)
            .and_then(|page| page.checked_mul(per_page))
            .and_then(|offset| i64::try_from(offset).ok())
            .ok_or_else(|| invalid("page is too large"))?;
        Ok(UsageParams {
            window,
            user,
            service,
            metric,
            sort,
            page,
            per_page,
            offset,
        })
    }
}

fn meter_filter(params: &UsageParams, include_service: bool) -> Document {
    let mut filter = doc! {
        "quantity": { "$ne": null },
        "created_at": { "$gte": bson::DateTime::from_chrono(params.window.from), "$lt": bson::DateTime::from_chrono(params.window.to) },
        "status": { "$in": ["finalized", "dead_letter"] },
        "$or": [{ "status": "finalized" }, { "status": "dead_letter", "forwarded": true }],
    };
    let mut clauses = Vec::new();
    if let Some(user) = &params.user {
        clauses.push(doc! { "$or": [{ "actor_user_id": user }, { "billing_owner_id": user }] });
    }
    if include_service && let Some(service) = &params.service {
        clauses.push(doc! { "$or": [{ "service_slug": service }, { "service_id": service }] });
    }
    if !clauses.is_empty() {
        filter.insert("$and", clauses);
    }
    filter
}

// Decimal128 avoids floating-point money arithmetic. Persisted quantities/rates
// are int64. Products below the int64 microcredit saturation limit need at most
// 25 significant digits and fit Decimal128 exactly; larger results clamp like
// billing::amounts::cost_micros.
fn legacy_cost(quantity: &str) -> Bson {
    doc! { "$convert": {
        "input": { "$min": [i64::MAX, { "$trunc": [{ "$divide": [
            { "$multiply": [ { "$toDecimal": quantity }, "$rate_pico" ] }, 1_000_000,
        ] }, 0] }] }, "to": "long", "onNull": 0,
    } }
    .into()
}

/// First group at the billing display's exact/legacy rate dimensions, plus
/// actor/owner/credential attribution. No per-row rate queries or Rust sums.
fn base_pipeline(params: &UsageParams) -> Vec<Document> {
    let mut group = doc! {
        "_id": {
            "actor": "$actor_user_id", "owner": "$billing_owner_id",
            "service_id": { "$ifNull": ["$service_id", null] },
            "service_slug": { "$ifNull": ["$service_slug", null] },
            "class": "$credential_class", "metric": "$metric", "code": "$lago_metric_code",
            "model": { "$ifNull": ["$model", null] }, "layer": "$layer",
            "api_key": { "$ifNull": ["$api_key_id", null] }, "acked": "$lago_acked",
            "billable": { "$ne": [{ "$ifNull": ["$wallet_id", null] }, null] },
        },
        "quantity": { "$sum": "$quantity" }, "events": { "$sum": 1 },
        "requests": { "$sum": { "$cond": ["$primary", 1, 0] } },
        "exact_cost_events": { "$sum": { "$cond": ["$exact", 1, 0] } },
        "legacy_cost_events": { "$sum": { "$cond": ["$exact", 0, 1] } },
        "legacy_quantity": { "$sum": { "$cond": ["$exact", 0, "$quantity"] } },
        "legacy_allowance_quantity": { "$sum": { "$cond": ["$exact", 0, { "$sum": "$funding.allowance_consumptions.quantity" }] } },
        "legacy_grant": { "$sum": { "$cond": ["$exact", 0, { "$sum": "$funding.grant_consumptions.amount_micros" }] } },
    };
    for field in TOKEN_FIELDS {
        group.insert(*field, doc! { "$sum": { "$cond": ["$primary", { "$ifNull": [format!("$token_breakdown.{field}"), 0] }, 0] } });
    }
    for (field, source) in COST_FIELDS.iter().zip([
        "total_charge_micros",
        "wallet_funded_micros",
        "grant_funded_micros",
        "allowance_funded_micros",
    ]) {
        group.insert(
            *field,
            doc! { "$sum": { "$toDecimal": { "$ifNull": [format!("$funding.{source}"), 0] } } },
        );
    }
    let mut costs = Document::new();
    for (field, legacy) in COST_FIELDS.iter().zip([
        Bson::String("$legacy_gross".into()),
        doc! { "$max": [0, { "$subtract": [{ "$subtract": ["$legacy_gross", "$legacy_allowance"] }, "$legacy_grant"] }] }.into(),
        Bson::String("$legacy_grant".into()), Bson::String("$legacy_allowance".into()),
    ]) {
        let sum: Bson = doc! { "$add": [format!("${field}"), legacy] }.into();
        let billable: Bson = if *field == "grant_cost_micros" { sum } else {
            doc! { "$cond": ["$unknown", null, sum] }.into()
        };
        costs.insert(*field, doc! { "$cond": ["$_id.billable", billable, 0] });
    }
    costs.insert(
        "unknown_cost_events",
        doc! { "$cond": ["$unknown", "$legacy_cost_events", 0] },
    );
    vec![
        doc! { "$match": meter_filter(params, true) },
        doc! { "$set": {
            "primary": { "$and": [{ "$eq": ["$layer", "platform"] }, { "$eq": ["$transaction_id", { "$concat": ["$billing_request_id", ":platform"] }] }] },
            "exact": { "$ne": [{ "$ifNull": ["$funding.total_charge_micros", null] }, null] },
        } },
        doc! { "$group": group },
        // _id equality lookups choose model-specific before generic. Only
        // historical billable groups need a cached rate, including stale rates
        // exactly as on /billing/usage.
        doc! { "$set": {
            "rate_ids": { "$cond": [ { "$and": ["$_id.billable", { "$gt": ["$legacy_quantity", 0] }] },
                [{ "$concat": ["$_id.code", ":", { "$ifNull": ["$_id.model", "*"] }] }, { "$concat": ["$_id.code", ":*"] }], [] ] },
        } },
        doc! { "$lookup": { "from": "billing_rate_cache", "localField": "rate_ids", "foreignField": "_id", "as": "rates" } },
        doc! { "$set": { "rate": { "$ifNull": [
            { "$arrayElemAt": [{ "$filter": { "input": "$rates", "as": "rate", "cond": { "$eq": ["$$rate._id", { "$arrayElemAt": ["$rate_ids", 0] }] } } }, 0] },
            { "$arrayElemAt": ["$rates", 0] },
        ] } } },
        doc! { "$set": {
            "rate_pico": { "$toDecimal": { "$ifNull": ["$rate.credits_per_unit_pico", { "$multiply": [{ "$toDecimal": { "$ifNull": ["$rate.credits_per_unit_micros", 0] } }, 1_000_000] }] } },
            "unknown": { "$and": ["$_id.billable", { "$gt": ["$legacy_quantity", 0] }, { "$eq": [{ "$size": "$rates" }, 0] }] },
        } },
        doc! { "$set": { "legacy_gross": legacy_cost("$legacy_quantity"), "legacy_allowance": legacy_cost("$legacy_allowance_quantity") } },
        doc! { "$set": costs },
    ]
}

fn sum_fields(group: &mut Document, source_prefix: &str) {
    for field in COUNT_FIELDS.iter().chain(TOKEN_FIELDS) {
        group.insert(*field, doc! { "$sum": format!("{source_prefix}{field}") });
    }
    for field in COST_FIELDS {
        // Decimal accumulation preserves integer micros even when a platform
        // total exceeds int64; clamp only at the response boundary.
        group.insert(*field, doc! { "$sum": { "$toDecimal": { "$ifNull": [format!("{source_prefix}{field}"), 0] } } });
        group.insert(format!("{field}_known"), doc! { "$sum": { "$cond": [{ "$ne": [{ "$ifNull": [format!("{source_prefix}{field}"), null] }, null] }, 1, 0] } });
    }
}

/// Two-stage metric/entity reduction keeps unlike units separate. Distinct sets
/// contain users/services, never individual request IDs (primary IDs are unique).
fn rollup(dimensions: &[&str]) -> Vec<Document> {
    let entity: Document = dimensions
        .iter()
        .map(|field| (field.to_string(), Bson::String(format!("$_id.{field}"))))
        .collect();
    let mut metric = doc! {
        "_id": { "entity": entity, "metric": "$_id.metric" },
        "quantity": { "$sum": "$quantity" },
        "users": { "$addToSet": "$_id.actor" },
        "services": { "$addToSet": { "$ifNull": ["$_id.service_id", "$_id.service_slug"] } },
    };
    sum_fields(&mut metric, "$");
    let mut entity = doc! {
        "_id": "$_id.entity",
        "quantities": { "$push": { "k": "$_id.metric", "v": "$quantity" } },
        "users": { "$push": "$users" }, "services": { "$push": "$services" },
    };
    sum_fields(&mut entity, "$");
    // Preserve nulls across the metric reduction, rather than counting its
    // $sum's synthetic zero as a known cost.
    for field in COST_FIELDS {
        entity.insert(
            format!("{field}_known"),
            doc! { "$sum": format!("${field}_known") },
        );
    }
    let mut project = doc! {
        "_id": 1, "quantities": { "$arrayToObject": "$quantities" },
        "total_tokens": { "$add": ["$prompt_tokens", "$completion_tokens"] },
    };
    for field in COUNT_FIELDS.iter().chain(TOKEN_FIELDS) {
        project.insert(*field, 1);
    }
    for field in COST_FIELDS {
        project.insert(*field, doc! { "$cond": [{ "$gt": [format!("${field}_known"), 0] }, { "$toLong": { "$min": [i64::MAX, format!("${field}")] } }, null] });
    }
    for field in ["users", "services"] {
        project.insert(format!("unique_{field}"), doc! { "$size": { "$reduce": {
            "input": format!("${field}"), "initialValue": [], "in": { "$setUnion": ["$$value", "$$this"] },
        } } });
    }
    vec![
        doc! { "$group": metric },
        doc! { "$group": entity },
        doc! { "$project": project },
    ]
}

fn summary_pipeline(params: &UsageParams) -> Vec<Document> {
    let mut pipeline = base_pipeline(params);
    pipeline.push(doc! { "$facet": {
        "totals": rollup(&[]),
        "services": rollup(&["service_id", "service_slug"]),
        "classes": rollup(&["class"]),
        "service_classes": rollup(&["service_id", "service_slug", "class"]),
    } });
    pipeline
}

fn ranking_pipeline(params: &UsageParams) -> Vec<Document> {
    let mut pipeline = base_pipeline(params);
    pipeline.extend(rollup(&["actor", "owner", "service_id", "service_slug"]));
    let sort = match params.sort.as_str() {
        "quantity" => format!("quantities.{}", params.metric),
        "cost" => "gross_cost_micros".into(),
        value => value.into(),
    };
    pipeline.push(doc! { "$facet": {
        "ranking": [
            { "$sort": { sort: -1, "_id.actor": 1, "_id.owner": 1, "_id.service_slug": 1, "_id.service_id": 1 } },
            { "$skip": params.offset }, { "$limit": params.per_page as i64 },
        ],
        "total": [{ "$count": "count" }],
    } });
    pipeline
}

fn query_error(error: mongodb::error::Error) -> AppError {
    if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 50)
    {
        AppError::AdminUsageQueryTimeout
    } else {
        error.into()
    }
}

async fn aggregate(db: &mongodb::Database, pipeline: Vec<Document>) -> AppResult<Vec<Document>> {
    db.collection::<Document>(COLLECTION_NAME)
        .aggregate(pipeline)
        .max_time(QUERY_TIMEOUT)
        .allow_disk_use(true)
        .await
        .map_err(query_error)?
        .try_collect()
        .await
        .map_err(query_error)
}

fn documents(document: &Document, key: &str) -> AppResult<Vec<Document>> {
    document
        .get_array(key)
        .map_err(|error| AppError::Internal(error.to_string()))?
        .iter()
        .map(|value| {
            value
                .as_document()
                .cloned()
                .ok_or_else(|| AppError::Internal("Invalid usage aggregation result".into()))
        })
        .collect()
}

fn stats(document: &Document) -> AppResult<UsageStats> {
    bson::from_document(document.clone()).map_err(|error| AppError::Internal(error.to_string()))
}
fn id(document: &Document) -> AppResult<&Document> {
    document
        .get_document("_id")
        .map_err(|error| AppError::Internal(error.to_string()))
}
fn service_key(document: &Document) -> (Option<String>, Option<String>) {
    (
        document.get_str("service_id").ok().map(str::to_string),
        document.get_str("service_slug").ok().map(str::to_string),
    )
}

pub async fn get_usage(
    db: &mongodb::Database,
    params: UsageParams,
) -> AppResult<AdminUsageResponse> {
    // A wall-clock bound covers cursor iteration and enrichment as well as the
    // server-side maxTimeMS on each aggregation.
    tokio::time::timeout(
        QUERY_TIMEOUT + Duration::from_secs(2),
        get_usage_inner(db, params),
    )
    .await
    .map_err(|_| AppError::AdminUsageQueryTimeout)?
}

async fn get_usage_inner(
    db: &mongodb::Database,
    params: UsageParams,
) -> AppResult<AdminUsageResponse> {
    let options = vec![
        doc! { "$match": meter_filter(&params, false) },
        doc! { "$group": { "_id": { "service_id": { "$ifNull": ["$service_id", null] }, "service_slug": { "$ifNull": ["$service_slug", null] } } } },
        doc! { "$sort": { "_id.service_slug": 1, "_id.service_id": 1 } },
    ];
    let (summary, ranking, options) = tokio::try_join!(
        aggregate(db, summary_pipeline(&params)),
        aggregate(db, ranking_pipeline(&params)),
        aggregate(db, options),
    )?;
    let summary = summary
        .first()
        .ok_or_else(|| AppError::Internal("Missing usage summary".into()))?;
    let ranking = ranking
        .first()
        .ok_or_else(|| AppError::Internal("Missing usage ranking".into()))?;
    let rank_rows = documents(ranking, "ranking")?;
    let service_rows = documents(summary, "services")?;
    let mut user_ids = HashSet::new();
    if let Some(user) = &params.user {
        user_ids.insert(user.clone());
    }
    for row in &rank_rows {
        for field in ["actor", "owner"] {
            if let Ok(value) = id(row)?.get_str(field) {
                user_ids.insert(value.to_string());
            }
        }
    }
    let mut service_ids = HashSet::new();
    let mut service_slugs = HashSet::new();
    for row in options
        .iter()
        .chain(service_rows.iter())
        .chain(rank_rows.iter())
    {
        let (sid, slug) = service_key(id(row)?);
        if let Some(sid) = sid {
            service_ids.insert(sid);
        }
        if let Some(slug) = slug {
            service_slugs.insert(slug);
        }
    }
    // Exactly one batched, projected user lookup, including selected filter and
    // org owners. No full User/model is serialized into the API.
    let users = async {
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find(doc! { "_id": { "$in": user_ids.into_iter().collect::<Vec<_>>() } })
            .projection(doc! { "display_name": 1, "email": 1, "user_type": 1 })
            .max_time(QUERY_TIMEOUT)
            .await?
            .try_collect::<Vec<Document>>()
            .await
    };
    let services = async {
        db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
            .find(doc! { "$or": [{ "_id": { "$in": service_ids.into_iter().collect::<Vec<_>>() } }, { "slug": { "$in": service_slugs.into_iter().collect::<Vec<_>>() } }] })
            .projection(doc! { "name": 1, "slug": 1 }).max_time(QUERY_TIMEOUT)
            .await?.try_collect::<Vec<Document>>().await
    };
    let (users, services) = tokio::try_join!(users, services).map_err(query_error)?;
    let users: HashMap<_, _> = users
        .into_iter()
        .filter_map(|user| Some((user.get_str("_id").ok()?.to_string(), user)))
        .collect();
    let identity = |uid: &str| {
        let user = users.get(uid);
        UsageIdentity {
            id: uid.into(),
            display_name: user
                .and_then(|u| u.get_str("display_name").ok())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(if user.is_some() {
                    "Unnamed user"
                } else {
                    "Unknown user"
                })
                .into(),
            email: user
                .and_then(|u| u.get_str("email").ok())
                .map(str::to_string),
            user_type: user
                .and_then(|u| u.get_str("user_type").ok())
                .unwrap_or(if user.is_some() { "person" } else { "unknown" })
                .into(),
        }
    };
    let mut services_by_ref = HashMap::new();
    for service in services {
        let value = UsageServiceIdentity {
            service_id: service.get_str("_id").ok().map(str::to_string),
            service_slug: service.get_str("slug").ok().map(str::to_string),
            service_name: service.get_str("name").unwrap_or("Unknown service").into(),
        };
        for key in [&value.service_id, &value.service_slug]
            .into_iter()
            .flatten()
        {
            services_by_ref.insert(key.clone(), value.clone());
        }
    }
    let service_identity = |key: &Document| {
        let (service_id, service_slug) = service_key(key);
        let known = service_id
            .as_ref()
            .and_then(|id| services_by_ref.get(id))
            .or_else(|| {
                service_slug
                    .as_ref()
                    .and_then(|slug| services_by_ref.get(slug))
            });
        UsageServiceIdentity {
            service_id,
            service_slug: service_slug.or_else(|| known.and_then(|s| s.service_slug.clone())),
            service_name: known
                .map(|s| s.service_name.clone())
                .unwrap_or_else(|| "Unknown service".into()),
        }
    };
    let class_row = |row: &Document| -> AppResult<UsageCredentialClass> {
        Ok(UsageCredentialClass {
            credential_class: id(row)?.get_str("class").unwrap_or("unknown").into(),
            usage: stats(row)?,
        })
    };
    let mut service_classes: HashMap<_, Vec<_>> = HashMap::new();
    for row in documents(summary, "service_classes")? {
        service_classes
            .entry(service_key(id(&row)?))
            .or_default()
            .push(class_row(&row)?);
    }
    let mut by_service = Vec::new();
    for row in service_rows {
        by_service.push(UsageService {
            service: service_identity(id(&row)?),
            usage: stats(&row)?,
            by_credential_class: service_classes
                .remove(&service_key(id(&row)?))
                .unwrap_or_default(),
        });
    }
    by_service.sort_by(|a, b| {
        a.service
            .service_slug
            .cmp(&b.service.service_slug)
            .then(a.service.service_id.cmp(&b.service.service_id))
    });
    let mut rows = Vec::new();
    for row in rank_rows {
        let key = id(&row)?;
        let actor = key.get_str("actor").unwrap_or("");
        let owner = key.get_str("owner").unwrap_or(actor);
        rows.push(UsageRanking {
            user: identity(actor),
            billing_owner: (owner != actor).then(|| identity(owner)),
            service: service_identity(key),
            usage: stats(&row)?,
        });
    }
    let totals = documents(summary, "totals")?
        .first()
        .map(stats)
        .transpose()?
        .unwrap_or_default();
    let ranking_total = documents(ranking, "total")?
        .first()
        .and_then(|row| row.get("count"))
        .and_then(|value| match value {
            Bson::Int32(n) => Some(i64::from(*n)),
            Bson::Int64(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0);
    Ok(AdminUsageResponse {
        window: params.window,
        totals,
        by_service,
        by_credential_class: documents(summary, "classes")?
            .iter()
            .map(class_row)
            .collect::<AppResult<_>>()?,
        ranking: rows,
        ranking_total,
        page: params.page,
        per_page: params.per_page,
        ranking_metric: params.metric,
        services: options
            .iter()
            .map(|row| id(row).map(&service_identity))
            .collect::<AppResult<_>>()?,
        selected_user: params.user.as_deref().map(identity),
    })
}

#[cfg(test)]
mod tests;
