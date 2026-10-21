//! Model executions by NyxBot and specialists, over the shared journal-validated
//! reporting engine. Orchestration and ordinary tools are separate populations.
use super::*;

const TOP_MODELS: i64 = 20;
const TOP_ACCOUNTS: i64 = 10;

pub(super) fn attribution_stages() -> Vec<Document> {
    let mut stages = vec![
        doc! { "$set": { "legacy_service": { "$cond": [{ "$eq": [{ "$ifNull": ["$_id.assistant_source", null] }, null] }, ["$_id.service_id"], []] }, "recovery_key": { "$cond": [
            { "$and": [{ "$eq": [{ "$ifNull": ["$_id.assistant_source", null] }, null] }, { "$ne": [{ "$ifNull": ["$_id.api_key", null] }, null] }] },
            ["$_id.api_key"], [],
        ] } } },
        // This server-owned view projects metadata before graph traversal.
        // Modern provenance starts with [] and performs no lineage walk.
        doc! { "$graphLookup": { "from": "usage_api_key_lineage", "startWith": "$recovery_key", "connectFromField": "rotation_predecessor_id", "connectToField": "_id", "as": "ancestors", "maxDepth": 31 } },
        doc! { "$graphLookup": { "from": "usage_api_key_lineage", "startWith": "$recovery_key", "connectFromField": "_id", "connectToField": "rotation_predecessor_id", "as": "descendants", "maxDepth": 31, "restrictSearchWithMatch": { "rotation_predecessor_id": { "$type": "string" } } } },
        doc! { "$set": { "lineage": { "$concatArrays": ["$ancestors", "$descendants"] }, "proof_keys": { "$setUnion": ["$recovery_key", "$ancestors._id", "$descendants._id"] } } },
    ];
    stages.extend([
        doc! { "$lookup": { "from": "assistant_agent_credentials", "localField": "proof_keys", "foreignField": "api_key_id", "as": "credentials", "let": { "actor": "$_id.actor" },
            "pipeline": [{ "$match": { "$expr": { "$eq": ["$user_id", "$$actor"] } } }, { "$project": { "_id": 0, "user_id": 1 } }, { "$limit": 1 }] } },
        doc! { "$lookup": { "from": "assistant_conversations", "localField": "proof_keys", "foreignField": "credential_api_key_id", "as": "conversations", "let": { "actor": "$_id.actor" },
            "pipeline": [{ "$match": { "$expr": { "$eq": ["$user_id", "$$actor"] } } }, { "$project": { "_id": 0, "user_id": 1 } }, { "$limit": 1 }] } },
        doc! { "$lookup": { "from": "audit_log", "localField": "proof_keys", "foreignField": "api_key_id", "as": "audits", "let": { "actor": "$_id.actor" },
            "pipeline": [{ "$match": { "$expr": { "$eq": ["$user_id", "$$actor"] }, "event_type": { "$in": ["assistant_agent_credential_provisioned", "assistant_agent_credential_replaced"] } } }, { "$project": { "_id": 0, "user_id": 1 } }, { "$limit": 1 }] } },
        doc! { "$set": { "verified_assistant": { "$or": [
            { "$in": ["$_id.assistant_source", ["model", "tool", "orchestration"]] },
            { "$and": [
                { "$eq": [{ "$ifNull": ["$_id.assistant_source", null] }, null] },
                { "$ne": [{ "$ifNull": ["$_id.api_key", null] }, null] },
                { "$in": ["$_id.actor", { "$concatArrays": ["$credentials.user_id", "$conversations.user_id", "$audits.user_id", { "$map": { "input": { "$filter": { "input": "$lineage", "as": "key", "cond": { "$eq": ["$$key.assistant_usage", true] } } }, "as": "key", "in": "$$key.user_id" } }] }] },
            ] },
        ] } } },
        doc! { "$lookup": { "from": "downstream_services", "localField": "legacy_service", "foreignField": "_id", "as": "inference_service", "pipeline": [{ "$match": { "$or": [{ "inference": { "$ne": null } }, { "slug": { "$regex": "^llm-" }, "provider_config_id": { "$ne": null } }] } }, { "$project": { "_id": 1 } }, { "$limit": 1 }] } },
        doc! { "$set": { "_id.assistant_population": { "$switch": {
            "branches": [
                { "case": { "$eq": ["$_id.assistant_source", "orchestration"] }, "then": "orchestration" },
                { "case": { "$and": ["$verified_assistant", { "$eq": ["$_id.service_slug", "llm-nyx"] }] }, "then": "orchestration" },
                { "case": { "$and": ["$verified_assistant", { "$or": [
                    { "$eq": ["$_id.assistant_source", "model"] },
                    { "$and": [{ "$eq": [{ "$ifNull": ["$_id.assistant_source", null] }, null] }, { "$gt": [{ "$size": "$inference_service" }, 0] }, { "$or": [
                                                { "$in": ["$_id.metric", ["tokens", "input_tokens", "output_tokens", "cached_tokens", "cache_creation_tokens", "images"]] },
                        { "$gt": [{ "$add": ["$prompt_tokens", "$completion_tokens"] }, 0] },
                    ] }] },
                ] }] }, "then": "model" },
                { "case": { "$and": ["$verified_assistant", { "$eq": [{ "$ifNull": ["$_id.assistant_source", null] }, null] }] }, "then": "unclassified" },
                { "case": "$verified_assistant", "then": "tool" },
                { "case": { "$eq": [{ "$ifNull": ["$_id.assistant_source", null] }, null] }, "then": "unattributable_legacy" },
            ], "default": "other",
        } } } },
        doc! { "$unset": ["ancestors", "descendants", "lineage", "recovery_key", "proof_keys", "credentials", "conversations", "audits", "verified_assistant", "legacy_service", "inference_service"] },
    ]);
    stages
}

pub(super) fn facets(
    _params: &UsageParams,
    reduced: &impl Fn(&[&str]) -> Vec<Document>,
) -> Document {
    let scoped = |dimensions: &[&str]| {
        let mut stages = vec![doc! { "$match": { "_id.assistant_population": "model" } }];
        stages.extend(reduced(dimensions));
        stages
    };
    let sort = doc! { "$sort": { "gross_cost": -1, "requests": -1, "_id": 1 } };
    let mut models = scoped(&["model"]);
    models.push(sort.clone());
    let mut model_top = models.clone();
    model_top.push(doc! { "$limit": TOP_MODELS });
    let mut model_other = models;
    model_other.extend([
        doc! { "$skip": TOP_MODELS },
        sum_stats_group(),
        mask_costs(),
    ]);
    // Account totals and their selected models come from one grouping/ranking.
    let mut accounts = scoped(&["owner", "model"]);
    let mut group = sum_group("$_id.owner".into());
    group.insert("models", doc! { "$topN": { "n": TOP_MODELS, "sortBy": { "gross_cost": -1, "requests": -1, "_id.model": 1 }, "output": "$$ROOT" } });
    accounts.extend([doc! { "$group": group }, mask_costs(), sort.clone()]);
    let mut account_top = accounts.clone();
    account_top.push(doc! { "$limit": TOP_ACCOUNTS });
    // Compute remainders before decoding/clamping counts. Cost presence is
    // subtracted separately: an unknown-only remainder has null cost.
    let mut other = doc! { "groups": { "$subtract": ["$groups", { "$size": "$models" }] } };
    for field in COUNT_FIELDS
        .iter()
        .chain(TOKEN_FIELDS)
        .chain([&"total_tokens"])
        .chain(COST_FIELDS)
    {
        let difference = doc! { "$subtract": [{ "$ifNull": [format!("${field}"), 0] }, { "$sum": format!("$models.{field}") }] };
        other.insert(
            *field,
            if COST_FIELDS.contains(field) {
                Bson::Document(difference)
            } else {
                doc! { "$max": [0_i64, difference] }.into()
            },
        );
    }
    for field in COST_FIELDS {
        other.insert(format!("{field}_known"), doc! { "$subtract": [format!("${field}_known"), { "$size": { "$filter": { "input": "$models", "as": "model", "cond": { "$ne": [{ "$ifNull": [format!("$$model.{field}"), null] }, null] } } } }] });
    }
    account_top.push(doc! { "$set": { "other": other } });
    let mut account_other = accounts;
    account_other.extend([
        doc! { "$skip": TOP_ACCOUNTS },
        sum_stats_group(),
        mask_costs(),
    ]);
    // Full-period membership stays fixed across daily buckets. Each model has
    // at most 32 daily points; the final result has at most 21 x 32 rows.
    let mut daily = scoped(&["model", "bucket"]);
    let mut day_group = sum_group("$_id.model".into());
    day_group.insert("days", doc! { "$push": "$$ROOT" });
    daily.extend([
        doc! { "$group": day_group }, mask_costs(),
        // $documentNumber permits one sort field. BSON compares these fixed-order
        // object fields lexicographically; exact Decimal128 stays numeric.
        doc! { "$set": { "series_order": { "unknown": { "$cond": [{ "$eq": [{ "$ifNull": ["$gross_cost", null] }, null] }, 1, 0] }, "cost": { "$multiply": [{ "$ifNull": ["$gross_cost", 0] }, -1] }, "requests": { "$multiply": ["$requests", -1] }, "model": "$_id" } } },
        doc! { "$setWindowFields": { "sortBy": { "series_order": 1 }, "output": { "position": { "$documentNumber": {} } } } },
        doc! { "$unwind": "$days" },
        doc! { "$set": { "days._id.is_other": { "$gt": ["$position", TOP_MODELS] }, "days._id.model": { "$cond": [{ "$gt": ["$position", TOP_MODELS] }, null, "$_id"] } } },
        doc! { "$replaceWith": "$days" },
        { let mut group = sum_group("$_id".into()); group.insert("groups", doc! { "$sum": 1_i64 }); doc! { "$group": group } },
        mask_costs(), doc! { "$sort": { "_id.bucket": 1, "_id.is_other": 1, "_id.model": 1 } },
    ]);
    let mut excluded = reduced(&["assistant_population"]);
    excluded.insert(0, doc! { "$match": { "_id.assistant_population": { "$in": ["orchestration", "tool", "unclassified", "unattributable_legacy"] } } });
    doc! { "totals": scoped(&[]), "models": model_top, "models_other": model_other,
        "accounts": account_top, "accounts_other": account_other, "daily": daily,
        "excluded": excluded,
        "freshness": [{ "$group": { "_id": null, "tail_rows": { "$sum": "$tail_rows" } } }],
    }
}

fn mask_costs() -> Document {
    let mut set = Document::new();
    for field in COST_FIELDS {
        set.insert(*field, doc! { "$cond": [{ "$gt": [format!("${field}_known"), 0] }, format!("${field}"), null] });
    }
    doc! { "$set": set }
}
fn number(row: &Document, field: &str) -> i64 {
    match row.get(field) {
        Some(Bson::Int64(n)) => *n,
        Some(Bson::Int32(n)) => i64::from(*n),
        Some(Bson::Double(n)) if n.is_finite() => *n as i64,
        _ => 0,
    }
}
fn sum_group(id: Bson) -> Document {
    let mut group = doc! { "_id": id, "groups": { "$sum": 1_i64 }, "total_tokens": { "$sum": "$total_tokens" } };
    sum_fields(&mut group, "$");
    group
}
fn sum_stats_group() -> Document {
    doc! { "$group": sum_group(Bson::Null) }
}

/// Only measures actually computed for this report. No fabricated quantities,
/// distinct counts, or legacy money projections are exposed.
#[derive(Debug, Default, Clone, Serialize, ToSchema)]
pub struct ModelStats {
    pub requests: i64,
    pub events: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cache_creation_tokens: i64,
    pub audio_input_tokens: i64,
    pub audio_output_tokens: i64,
    pub image_input_tokens: i64,
    pub image_output_tokens: i64,
    pub total_tokens: i64,
    pub gross_cost: Option<crate::models::credits::Credits>,
    pub wallet_cost: Option<crate::models::credits::Credits>,
    pub grant_cost: Option<crate::models::credits::Credits>,
    pub allowance_cost: Option<crate::models::credits::Credits>,
    pub exact_cost_events: i64,
    pub legacy_cost_events: i64,
    pub unknown_cost_events: i64,
}
fn model_stats(row: &Document) -> AppResult<ModelStats> {
    let cost = |field: &str| -> AppResult<Option<crate::models::credits::Credits>> {
        if row.contains_key(format!("{field}_known")) && number(row, &format!("{field}_known")) == 0
        {
            return Ok(None);
        }
        Ok(row
            .get(field)
            .filter(|v| !matches!(v, Bson::Null))
            .cloned()
            .map(|v| crate::models::credits::Credits::from_bson(v, 1))
            .transpose()?)
    };
    Ok(ModelStats {
        requests: number(row, "requests"),
        events: number(row, "events"),
        prompt_tokens: number(row, "prompt_tokens"),
        completion_tokens: number(row, "completion_tokens"),
        cached_tokens: number(row, "cached_tokens"),
        cache_creation_tokens: number(row, "cache_creation_tokens"),
        audio_input_tokens: number(row, "audio_input_tokens"),
        audio_output_tokens: number(row, "audio_output_tokens"),
        image_input_tokens: number(row, "image_input_tokens"),
        image_output_tokens: number(row, "image_output_tokens"),
        total_tokens: number(row, "total_tokens"),
        gross_cost: cost("gross_cost")?,
        wallet_cost: cost("wallet_cost")?,
        grant_cost: cost("grant_cost")?,
        allowance_cost: cost("allowance_cost")?,
        exact_cost_events: number(row, "exact_cost_events"),
        legacy_cost_events: number(row, "legacy_cost_events"),
        unknown_cost_events: number(row, "unknown_cost_events"),
    })
}
#[derive(Debug, Serialize, ToSchema)]
pub struct ModelUsage {
    pub model: Option<String>,
    pub is_other: bool,
    pub groups: i64,
    pub usage: ModelStats,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct DailyUsage {
    pub day: DateTime<Utc>,
    #[serde(flatten)]
    pub model: ModelUsage,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct AccountUsage {
    pub account: UsageIdentity,
    pub usage: ModelStats,
    pub models: Vec<ModelUsage>,
    pub other_model_count: i64,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct ExcludedUsage {
    pub population: String,
    pub usage: ModelStats,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct Response {
    pub window: UsageWindow,
    pub freshness: UsageFreshness,
    pub totals: ModelStats,
    pub models: Vec<ModelUsage>,
    pub daily: Vec<DailyUsage>,
    pub accounts: Vec<AccountUsage>,
    pub other_accounts: Option<ModelUsage>,
    pub excluded: Vec<ExcludedUsage>,
}
fn model_row(row: &Document, other: bool) -> AppResult<ModelUsage> {
    Ok(ModelUsage {
        model: row
            .get_document("_id")
            .ok()
            .and_then(|id| id.get_str("model").ok())
            .map(str::to_owned),
        is_other: other,
        groups: if other { number(row, "groups") } else { 1 },
        usage: model_stats(row)?,
    })
}
pub async fn get(db: &mongodb::Database, mut params: UsageParams) -> AppResult<Response> {
    params.nyxbot = true;
    params.analytics = Some(analytics::AnalyticsOptions {
        measure: "cost".into(),
        metric: "tokens".into(),
        breakdown: "model".into(),
        top: 20,
        granularity: "day",
    });
    tokio::time::timeout(
        QUERY_TIMEOUT + Duration::from_secs(2),
        get_inner(db, params),
    )
    .await
    .map_err(|_| AppError::AdminUsageQueryTimeout)?
}
async fn get_inner(db: &mongodb::Database, params: UsageParams) -> AppResult<Response> {
    let (result, freshness) = fast_aggregate(db, &params).await?;
    let mut models = documents(&result, "models")?
        .iter()
        .map(|row| model_row(row, false))
        .collect::<AppResult<Vec<_>>>()?;
    for row in documents(&result, "models_other")? {
        models.push(model_row(&row, true)?);
    }
    let rows = documents(&result, "accounts")?;
    let ids = rows
        .iter()
        .filter_map(|r| r.get_str("_id").ok().map(str::to_owned))
        .collect::<Vec<_>>();
    let identities = crate::services::reporting_identity_service::resolve(db, &ids, QUERY_TIMEOUT)
        .await
        .map_err(query_error)?;
    let mut accounts = Vec::new();
    for row in rows {
        let owner = row.get_str("_id").unwrap_or("");
        let identity = identities.get(owner);
        let mut models = documents(&row, "models")?
            .iter()
            .map(|m| model_row(m, false))
            .collect::<AppResult<Vec<_>>>()?;
        let other = row
            .get_document("other")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let other_model_count = number(other, "groups");
        if other_model_count > 0 {
            models.push(model_row(other, true)?);
        }
        accounts.push(AccountUsage {
            account: UsageIdentity {
                id: owner.into(),
                display_name: identity
                    .map(|i| i.label())
                    .unwrap_or("Unknown account")
                    .into(),
                email: identity.and_then(|i| i.email.clone()),
                user_type: identity
                    .map(|i| i.user_type.as_str())
                    .unwrap_or("unknown")
                    .into(),
            },
            usage: model_stats(&row)?,
            models,
            other_model_count,
        });
    }
    Ok(Response {
        window: params.window,
        freshness,
        totals: documents(&result, "totals")?
            .first()
            .map(model_stats)
            .transpose()?
            .unwrap_or_default(),
        models,
        daily: documents(&result, "daily")?
            .iter()
            .map(|row| {
                Ok(DailyUsage {
                    day: id(row)?
                        .get_datetime("bucket")
                        .map_err(|e| AppError::Internal(e.to_string()))?
                        .to_chrono(),
                    model: model_row(row, id(row)?.get_bool("is_other").unwrap_or(false))?,
                })
            })
            .collect::<AppResult<_>>()?,
        accounts,
        other_accounts: documents(&result, "accounts_other")?
            .first()
            .map(|r| model_row(r, true))
            .transpose()?,
        excluded: documents(&result, "excluded")?
            .iter()
            .map(|r| {
                Ok(ExcludedUsage {
                    population: id(r)?
                        .get_str("assistant_population")
                        .unwrap_or("unknown")
                        .into(),
                    usage: model_stats(r)?,
                })
            })
            .collect::<AppResult<_>>()?,
    })
}
