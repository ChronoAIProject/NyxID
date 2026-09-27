//! Bounded chart projections over the same rollup/tail snapshot as usage tables.
use super::*;
use chrono::{Datelike, Months};

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AnalyticsQuery {
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Comma-separated service ids/slugs, acting user ids, and billing owner ids.
    pub services: Option<String>,
    pub actors: Option<String>,
    pub owners: Option<String>,
    pub measure: Option<String>,
    pub metric: Option<String>,
    pub breakdown: Option<String>,
    pub top: Option<u32>,
    /// auto (default), hour, day, week (Monday UTC), or calendar month.
    pub interval: Option<String>,
}

#[derive(Default)]
pub struct UsageSelection {
    pub services: Vec<String>,
    pub actors: Vec<String>,
    pub owners: Vec<String>,
}

pub struct AnalyticsOptions {
    pub measure: String,
    pub metric: String,
    pub breakdown: String,
    pub top: u32,
    pub granularity: &'static str,
}

pub fn filter_values(value: Option<String>, ids: bool) -> AppResult<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut values = value
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if values.len() > 20 || values.iter().any(|v| v.is_empty() || v.len() > 256) {
        return Err(AppError::ValidationError(
            "Choose at most 20 nonempty filter values".into(),
        ));
    }
    if ids {
        for value in &mut values {
            *value = uuid::Uuid::parse_str(value)
                .map_err(|_| AppError::ValidationError("Identity filters must be UUIDs".into()))?
                .to_string();
        }
    }
    values.sort();
    values.dedup();
    Ok(values)
}

impl AnalyticsQuery {
    pub fn validate(self, now: DateTime<Utc>) -> AppResult<UsageParams> {
        let mut params = AdminUsageQuery {
            period: self.period,
            from: self.from,
            to: self.to,
            metric: self.metric.clone(),
            services: self.services,
            actors: self.actors,
            owners: self.owners,
            ..Default::default()
        }
        .validate(now)?;
        let measure = self.measure.unwrap_or_else(|| "cost".into());
        if ![
            "cost",
            "requests",
            "events",
            "exact_cost_events",
            "legacy_cost_events",
            "unknown_cost_events",
            "total_tokens",
            "prompt_tokens",
            "completion_tokens",
            "cached_tokens",
            "cache_creation_tokens",
            "quantity",
            "wallet_cost",
            "grant_cost",
            "allowance_cost",
        ]
        .contains(&measure.as_str())
        {
            return Err(AppError::ValidationError(
                "Unknown analytics measure".into(),
            ));
        }
        let breakdown = self.breakdown.unwrap_or_else(|| "service".into());
        if !["service", "user", "owner", "credential_class"].contains(&breakdown.as_str()) {
            return Err(AppError::ValidationError(
                "Unknown analytics breakdown".into(),
            ));
        }
        let top = self.top.unwrap_or(5);
        if ![0, 5, 10].contains(&top) {
            return Err(AppError::ValidationError(
                "top must be 0 (aggregate), 5, or 10".into(),
            ));
        }
        let granularity = match self.interval.as_deref().unwrap_or("auto") {
            "auto" => {
                if params.window.to - params.window.from <= chrono::Duration::hours(48) {
                    "hour"
                } else {
                    "day"
                }
            }
            "hour" => "hour",
            "day" => "day",
            "week" => "week",
            "month" => "month",
            _ => {
                return Err(AppError::ValidationError(
                    "Choose auto, hour, day, week, or month".into(),
                ));
            }
        };
        params.analytics = Some(AnalyticsOptions {
            measure,
            metric: params.metric.clone(),
            breakdown,
            top,
            granularity,
        });
        Ok(params)
    }
}

impl UsageSelection {
    pub(super) fn filter(&self, filter: &mut Document, actor: &str, owner: &str) {
        if !self.actors.is_empty() {
            filter.insert(actor, doc! { "$in": &self.actors });
        }
        if !self.owners.is_empty() {
            filter.insert(owner, doc! { "$in": &self.owners });
        }
    }
}

impl AnalyticsOptions {
    pub(super) fn bucket(&self, date: impl Into<Bson>) -> Bson {
        let mut truncate =
            doc! { "date": date.into(), "unit": self.granularity, "timezone": "UTC" };
        if self.granularity == "week" {
            truncate.insert("startOfWeek", "monday");
        }
        doc! { "$dateTrunc": truncate }.into()
    }
    fn field(&self) -> String {
        match self.measure.as_str() {
            "cost" => "gross_cost_micros".into(),
            "wallet_cost" => "wallet_cost_micros".into(),
            "grant_cost" => "grant_cost_micros".into(),
            "allowance_cost" => "allowance_cost_micros".into(),
            "quantity" => format!("quantities.{}", self.metric),
            value => value.into(),
        }
    }
    fn is_cost(&self) -> bool {
        self.measure == "cost" || self.measure.ends_with("_cost")
    }
    pub(super) fn facets(&self, reduced: &impl Fn(&[&str]) -> Vec<Document>) -> Document {
        let dimensions: &[&str] = match self.breakdown.as_str() {
            "user" => &["actor"],
            "owner" => &["owner"],
            "credential_class" => &["class"],
            _ => &["service_id", "service_slug"],
        };
        let mut distribution = reduced(if self.top == 0 { &[] } else { dimensions });
        let field = self.field();
        let measure: Bson = if self.is_cost() {
            format!("${field}").into()
        } else {
            doc! { "$ifNull": [format!("${field}"), 0_i64] }.into()
        };
        distribution.push(doc! { "$set": { "value": measure } });
        distribution.push(doc! { "$sort": { "value": -1, "_id": 1 } });
        let limit = i64::from(if self.top == 0 { 1 } else { self.top });
        let mut top = distribution.clone();
        top.push(doc! { "$limit": limit });
        let mut other = distribution;
        other.extend([
            doc! { "$skip": limit },
            doc! { "$group": {
                "_id": null, "value": { "$sum": { "$toDecimal": "$value" } },
                "known": { "$sum": { "$cond": [{ "$ne": [{ "$ifNull": ["$value", null] }, null] }, 1, 0] } },
                "unknown_cost_events": { "$sum": "$unknown_cost_events" }, "groups": { "$sum": 1 },
            } },
            doc! { "$set": { "value": { "$cond": [{ "$gt": ["$known", 0] }, { "$toLong": { "$min": [i64::MAX, "$value"] } }, null] } } },
        ]);
        let mut series_dimensions = if self.top == 0 {
            Vec::new()
        } else {
            dimensions.to_vec()
        };
        let entity: Document = series_dimensions
            .iter()
            .map(|field| ((*field).to_owned(), Bson::String(format!("$_id.{field}"))))
            .collect();
        series_dimensions.push("bucket");
        let mut series = reduced(&series_dimensions);
        let mut series_group = doc! {
            "_id": entity, "value": { "$sum": { "$toDecimal": format!("${field}") } },
            "known": { "$sum": { "$cond": [{ "$ne": [{ "$ifNull": [format!("${field}"), null] }, null] }, 1, 0] } },
            "points": { "$push": { "bucket": "$_id.bucket", "value": { "$ifNull": [format!("${field}"), Bson::Null] }, "requests": "$requests", "unknown_cost_events": "$unknown_cost_events" } },
        };
        if !self.is_cost() {
            series_group.insert("known", doc! { "$sum": 1 });
            series_group
                .get_document_mut("points")
                .expect("points")
                .get_document_mut("$push")
                .expect("point")
                .insert("value", doc! { "$ifNull": [format!("${field}"), 0_i64] });
        }
        series.extend([
            doc! { "$group": series_group },
            doc! { "$set": { "value": { "$cond": [{ "$gt": ["$known", 0] }, { "$toLong": { "$min": [i64::MAX, "$value"] } }, null] } } },
            doc! { "$sort": { "value": -1, "_id": 1 } },
        ]);
        let mut series_other = series.clone();
        series.push(doc! { "$limit": limit });
        series_other.extend([
            doc! { "$skip": limit }, doc! { "$unwind": "$points" },
            doc! { "$group": {
                "_id": "$points.bucket", "value": { "$sum": { "$toDecimal": "$points.value" } },
                "known": { "$sum": { "$cond": [{ "$ne": ["$points.value", null] }, 1, 0] } },
                "requests": { "$sum": "$points.requests" }, "unknown_cost_events": { "$sum": "$points.unknown_cost_events" },
            } },
            doc! { "$project": { "_id": 0, "bucket": "$_id", "requests": 1, "unknown_cost_events": 1,
                "value": { "$cond": [{ "$gt": ["$known", 0] }, { "$toLong": { "$min": [i64::MAX, "$value"] } }, null] },
            } },
        ]);
        let mut trend = reduced(&["bucket"]);
        trend.push(doc! { "$sort": { "_id.bucket": 1 } });
        doc! {
            "totals": reduced(&[]), "trend": trend, "top": top, "other": other, "series": series, "series_other": series_other,
            "freshness": [{ "$group": { "_id": null, "tail_rows": { "$sum": "$tail_rows" } } }],
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AnalyticsPoint {
    pub bucket: DateTime<Utc>,
    pub value: Option<i64>,
    pub requests: i64,
    pub unknown_cost_events: i64,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct AnalyticsSlice {
    pub id: Option<String>,
    pub label: String,
    pub value: Option<i64>,
    pub unknown_cost_events: i64,
    pub is_other: bool,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct AnalyticsSeries {
    pub label: String,
    pub is_other: bool,
    pub points: Vec<AnalyticsPoint>,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct AnalyticsResponse {
    pub window: UsageWindow,
    pub freshness: UsageFreshness,
    pub granularity: String,
    pub unit: String,
    pub total: Option<i64>,
    pub totals: UsageStats,
    pub points: Vec<AnalyticsPoint>,
    pub slices: Vec<AnalyticsSlice>,
    pub series: Vec<AnalyticsSeries>,
}

fn value(stats: &UsageStats, options: &AnalyticsOptions) -> Option<i64> {
    // A partial cost is not a total. Preserve a gap; totals still expose the
    // known subtotal and coverage counts for the UI's explicit estimate note.
    if options.is_cost() && stats.unknown_cost_events > 0 {
        return None;
    }
    match options.measure.as_str() {
        "cost" => stats.gross_cost_micros,
        "wallet_cost" => stats.wallet_cost_micros,
        "grant_cost" => stats.grant_cost_micros,
        "allowance_cost" => stats.allowance_cost_micros,
        "requests" => Some(stats.requests),
        "events" => Some(stats.events),
        "exact_cost_events" => Some(stats.exact_cost_events),
        "legacy_cost_events" => Some(stats.legacy_cost_events),
        "unknown_cost_events" => Some(stats.unknown_cost_events),
        "total_tokens" => Some(stats.total_tokens),
        "prompt_tokens" => Some(stats.prompt_tokens),
        "completion_tokens" => Some(stats.completion_tokens),
        "cached_tokens" => Some(stats.cached_tokens),
        "cache_creation_tokens" => Some(stats.cache_creation_tokens),
        "quantity" => Some(*stats.quantities.get(&options.metric).unwrap_or(&0)),
        _ => None,
    }
}

fn bucket_start(at: DateTime<Utc>, granularity: &str) -> DateTime<Utc> {
    use crate::services::billing::usage_rollup::{day, hour};
    match granularity {
        "hour" => hour(at),
        "week" => day(at) - chrono::Duration::days(i64::from(at.weekday().num_days_from_monday())),
        "month" => day(at).with_day(1).expect("first day exists"),
        _ => day(at),
    }
}

fn next_bucket(at: DateTime<Utc>, granularity: &str) -> Option<DateTime<Utc>> {
    match granularity {
        "hour" => at.checked_add_signed(chrono::Duration::hours(1)),
        "week" => at.checked_add_signed(chrono::Duration::weeks(1)),
        "month" => at.checked_add_months(Months::new(1)),
        _ => at.checked_add_signed(chrono::Duration::days(1)),
    }
}

pub async fn get_analytics(
    db: &mongodb::Database,
    params: UsageParams,
) -> AppResult<AnalyticsResponse> {
    tokio::time::timeout(
        QUERY_TIMEOUT + Duration::from_secs(2),
        get_inner(db, params),
    )
    .await
    .map_err(|_| AppError::AdminUsageQueryTimeout)?
}
async fn get_inner(db: &mongodb::Database, params: UsageParams) -> AppResult<AnalyticsResponse> {
    let options = params
        .analytics
        .as_ref()
        .ok_or_else(|| AppError::ValidationError("Analytics options required".into()))?;
    let (result, freshness) = fast_aggregate(db, &params).await?;
    let totals = documents(&result, "totals")?
        .first()
        .map(stats)
        .transpose()?
        .unwrap_or_default();
    let mut buckets = HashMap::new();
    for row in documents(&result, "trend")? {
        let bucket = id(&row)?
            .get_datetime("bucket")
            .map_err(|_| AppError::Internal("Invalid analytics bucket".into()))?
            .to_chrono();
        buckets.insert(bucket, stats(&row)?);
    }
    let mut points = Vec::new();
    let mut bucket = bucket_start(params.window.from, options.granularity);
    while bucket < params.window.to {
        let stats = buckets.remove(&bucket);
        points.push(AnalyticsPoint {
            bucket,
            value: stats
                .as_ref()
                .map_or(Some(0), |stats| value(stats, options)),
            requests: stats.as_ref().map_or(0, |stats| stats.requests),
            unknown_cost_events: stats.as_ref().map_or(0, |stats| stats.unknown_cost_events),
        });
        let Some(next) = next_bucket(bucket, options.granularity) else {
            break;
        };
        bucket = next;
    }
    let rows = documents(&result, "top")?;
    let group_key = |row: &Document| -> AppResult<Option<String>> {
        let key = id(row)?;
        Ok(match options.breakdown.as_str() {
            "user" => key.get_str("actor").ok().map(str::to_owned),
            "owner" => key.get_str("owner").ok().map(str::to_owned),
            "credential_class" => key.get_str("class").ok().map(str::to_owned),
            _ => {
                let (id, slug) = service_key(key);
                id.or(slug)
            }
        })
    };
    let keys = rows.iter().map(&group_key).collect::<AppResult<Vec<_>>>()?;
    let refs: Vec<_> = keys.iter().flatten().cloned().collect();
    let service = options.breakdown == "service";
    let labels: Vec<Document> = if options.breakdown == "credential_class" {
        Vec::new()
    } else {
        db.collection::<Document>(if service {
            "downstream_services"
        } else {
            "users"
        })
        .find(if service {
            doc! { "$or": [{ "_id": { "$in": &refs } }, { "slug": { "$in": &refs } }] }
        } else {
            doc! { "_id": { "$in": &refs } }
        })
        .projection(if service {
            doc! { "name": 1, "slug": 1 }
        } else {
            doc! { "display_name": 1, "email": 1 }
        })
        .max_time(QUERY_TIMEOUT)
        .await
        .map_err(query_error)?
        .try_collect()
        .await
        .map_err(query_error)?
    };
    let mut names = HashMap::new();
    if options.breakdown == "credential_class" {
        for (key, label) in [
            ("user_owned", "User's own key (BYOK)"),
            ("nyxid_managed_master", "NyxID platform key"),
            ("agent_override_user_owned", "Own key · agent override"),
            ("node_managed", "Own key · node-managed"),
            ("nyxid_platform_oauth_app", "Shared OAuth app"),
            ("no_auth", "No authentication"),
            ("unknown", "Unknown credential class"),
        ] {
            names.insert(key.to_owned(), label.to_owned());
        }
    }
    for row in labels {
        let label = if service {
            row.get_str("name").ok()
        } else {
            row.get_str("display_name")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| row.get_str("email").ok())
        };
        if let Some(label) = label {
            for key in [row.get_str("_id").ok(), row.get_str("slug").ok()]
                .into_iter()
                .flatten()
            {
                names.insert(key.to_owned(), label.to_owned());
            }
        }
    }
    let mut label_counts = HashMap::<String, usize>::new();
    for key in &refs {
        if let Some(label) = names.get(key) {
            *label_counts.entry(label.clone()).or_default() += 1;
        }
    }
    for key in &refs {
        if let Some(label) = names.get_mut(key)
            && (label_counts.get(label).copied().unwrap_or_default() > 1
                || matches!(label.as_str(), "Other" | "Unattributed"))
        {
            *label = format!("{label} ({key})");
        }
    }
    let mut slices = Vec::new();
    for (row, key) in rows.iter().zip(keys) {
        let stats = stats(row)?;
        let label = if options.top == 0 {
            "All usage".into()
        } else {
            key.as_ref()
                .and_then(|key| names.get(key))
                .cloned()
                .unwrap_or_else(|| key.clone().unwrap_or_else(|| "Unattributed".into()))
        };
        slices.push(AnalyticsSlice {
            id: key,
            label,
            value: value(&stats, options),
            unknown_cost_events: stats.unknown_cost_events,
            is_other: false,
        });
    }
    if let Some(other) = documents(&result, "other")?.first() {
        let unknown = number(other, "unknown_cost_events");
        let other_value = match other.get("value") {
            Some(Bson::Int64(n)) => Some(*n),
            Some(Bson::Int32(n)) => Some(i64::from(*n)),
            _ => None,
        };
        slices.push(AnalyticsSlice {
            id: None,
            label: "Other".into(),
            value: if options.is_cost() && unknown > 0 {
                None
            } else {
                other_value
            },
            unknown_cost_events: unknown,
            is_other: true,
        });
    }
    let fill_series = |rows: Vec<Document>| -> AppResult<Vec<AnalyticsPoint>> {
        let mut values = HashMap::new();
        for row in rows {
            let at = row
                .get_datetime("bucket")
                .map_err(|_| AppError::Internal("Invalid series bucket".into()))?
                .to_chrono();
            let unknown = number(&row, "unknown_cost_events");
            let amount = match row.get("value") {
                Some(Bson::Int64(n)) => Some(*n),
                Some(Bson::Int32(n)) => Some(i64::from(*n)),
                _ => None,
            };
            values.insert(
                at,
                AnalyticsPoint {
                    bucket: at,
                    value: if options.is_cost() && unknown > 0 {
                        None
                    } else {
                        amount
                    },
                    requests: number(&row, "requests"),
                    unknown_cost_events: unknown,
                },
            );
        }
        Ok(points
            .iter()
            .map(|point| {
                values.remove(&point.bucket).unwrap_or(AnalyticsPoint {
                    bucket: point.bucket,
                    value: Some(0),
                    requests: 0,
                    unknown_cost_events: 0,
                })
            })
            .collect())
    };
    let mut series = Vec::new();
    for row in documents(&result, "series")? {
        let key = group_key(&row)?;
        let label = if options.top == 0 {
            "All usage".into()
        } else {
            key.as_ref()
                .and_then(|id| names.get(id))
                .cloned()
                .unwrap_or_else(|| key.unwrap_or_else(|| "Unattributed".into()))
        };
        series.push(AnalyticsSeries {
            label,
            is_other: false,
            points: fill_series(documents(&row, "points")?)?,
        });
    }
    let other_points = documents(&result, "series_other")?;
    if !other_points.is_empty() {
        series.push(AnalyticsSeries {
            label: "Other".into(),
            is_other: true,
            points: fill_series(other_points)?,
        });
    }
    Ok(AnalyticsResponse {
        total: if totals.events == 0 {
            Some(0)
        } else {
            value(&totals, options)
        },
        window: params.window.clone(),
        freshness,
        granularity: options.granularity.into(),
        unit: if options.is_cost() {
            "microcredits".into()
        } else if options.measure == "quantity" {
            options.metric.clone()
        } else if options.measure.ends_with("_tokens") {
            "tokens".into()
        } else if options.measure == "events" || options.measure.ends_with("_events") {
            "events".into()
        } else {
            "requests".into()
        },
        totals,
        points,
        slices,
        series,
    })
}
fn number(row: &Document, field: &str) -> i64 {
    match row.get(field) {
        Some(Bson::Int64(n)) => *n,
        Some(Bson::Int32(n)) => i64::from(*n),
        _ => 0,
    }
}
