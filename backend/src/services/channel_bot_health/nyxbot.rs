//! Current binding metadata shared by bot Health and assistant discovery.
use crate::{
    errors::{AppError, AppResult},
    models::channel_bot::ChannelBot,
};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Serialize;

/// Closed metadata only. Both writes and projections use this constructor;
/// unknown historical fields/prose cannot become a public diagnostic.
#[derive(Clone, Debug, Serialize)]
pub struct SetupDiagnostic {
    pub operation: &'static str,
    pub outcome: &'static str,
    pub http_status: Option<u16>,
    pub field: Option<&'static str>,
    pub problem_code: Option<&'static str>,
}

impl SetupDiagnostic {
    fn operation(value: &str) -> Option<&'static str> {
        match value {
            "configuration" => Some("configuration"),
            "catalog_read" => Some("catalog_read"),
            "create" => Some("create"),
            "channel_update" => Some("channel_update"),
            "routing_update" => Some("routing_update"),
            _ => None,
        }
    }
    fn field(value: &str) -> Option<&'static str> {
        match value {
            "/name" => Some("/name"),
            "/profile/metadata/name" => Some("/profile/metadata/name"),
            "/profile/metadata/provider" => Some("/profile/metadata/provider"),
            "/sources/0/issuer" => Some("/sources/0/issuer"),
            "/sources/0/bot_username" => Some("/sources/0/bot_username"),
            "/sources/0/bot_id" => Some("/sources/0/bot_id"),
            "/credentials/agent_key" => Some("/credentials/agent_key"),
            "/credentials/channel_key" => Some("/credentials/channel_key"),
            "/routing" => Some("/routing"),
            _ => None,
        }
    }
    fn status(value: u16) -> Option<u16> {
        matches!(
            value,
            400 | 401 | 403 | 404 | 409 | 422 | 429 | 500 | 502 | 503 | 504
        )
        .then_some(value)
    }
    pub(crate) fn new(
        operation: &str,
        response: Option<&crate::services::nyxbot_gateway_service::GatewayResponse>,
    ) -> Option<Self> {
        let operation = Self::operation(operation)?;
        Some(Self {
            operation,
            outcome: if matches!(operation, "configuration" | "catalog_read") {
                "not_sent"
            } else if response.is_some_and(|r| (400..500).contains(&r.status)) {
                "rejected"
            } else {
                "unknown"
            },
            http_status: response.and_then(|r| Self::status(r.status)),
            problem_code: response
                .and_then(|r| r.body["code"].as_str())
                .and_then(crate::services::nyxbot_gateway_service::setup_problem_code),
            // CMA's Invalid(ValidationError) serializes "{pointer}: {message}".
            // Only that exact pointer position, under invalid_request/400, counts.
            field: response
                .filter(|r| r.status == 400 && r.body["code"] == "invalid_request")
                .and_then(|r| r.body["detail"].as_str())
                .and_then(|detail| detail.split_once(": "))
                .and_then(|(pointer, _)| Self::field(pointer)),
        })
    }
    pub(crate) fn from_document(doc: &bson::Document) -> Option<Self> {
        Some(Self {
            operation: Self::operation(doc.get_str("operation").ok()?)?,
            outcome: match doc.get_str("outcome").ok()? {
                "not_sent" => "not_sent",
                "rejected" => "rejected",
                "unknown" => "unknown",
                _ => return None,
            },
            http_status: doc
                .get_i32("http_status")
                .ok()
                .and_then(|s| u16::try_from(s).ok())
                .and_then(Self::status),
            field: doc.get_str("field").ok().and_then(Self::field),
            problem_code: doc
                .get_str("problem_code")
                .ok()
                .and_then(crate::services::nyxbot_gateway_service::setup_problem_code),
        })
    }
}

pub(crate) fn current_gateway(
    bot: &ChannelBot,
    row: Option<&NyxbotHealthRow>,
    fallback: bool,
) -> bool {
    if !matches!(bot.platform.as_str(), "telegram" | "telegram-new") {
        return fallback;
    }
    row.is_some_and(|row| {
        row.status == "active" && matches!(row.transport.as_str(), "" | "gateway")
    })
}

pub(crate) fn gateway_failure<'a>(bot: &ChannelBot, row: &'a NyxbotHealthRow) -> Option<&'a str> {
    if !matches!(bot.platform.as_str(), "telegram" | "telegram-new")
        || row.status != "failed"
        || !matches!(row.transport.as_str(), "" | "gateway")
    {
        return None;
    }
    row.last_error.as_deref().filter(|code| {
        !crate::services::assistant_wake_service::text::gateway_error_guidance(code).is_empty()
    })
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct NyxbotHealthRow {
    #[serde(default)]
    pub(crate) answer_events: Vec<crate::services::channel_bot_health::answer::Observation>,
    #[serde(default)]
    pub(crate) answer_responses: Vec<crate::services::channel_bot_health::answer::Observation>,
    #[serde(rename = "_id")]
    pub(crate) id: String,
    pub(crate) user_id: String,
    pub(crate) channel_bot_id: String,
    #[serde(default)]
    pub(crate) bot_owner_id: Option<String>,
    #[serde(default)]
    pub(crate) transport: String,
    #[serde(default)]
    pub(crate) setup_diagnostic: Option<bson::Document>,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) org_specialist: Option<crate::models::org_channel::Binding>,
    #[serde(default)]
    pub(crate) agent_id: Option<String>,
    #[serde(default)]
    pub(crate) org_restore_last_reason: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub(crate) org_restore_next_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub(crate) org_restore_claim_id: Option<String>,
    #[serde(default)]
    pub(crate) org_restore_attempts: i32,
    #[serde(default)]
    pub(crate) native_intake_refused: bool,
    #[serde(default)]
    pub(crate) gateway_not_received:
        Option<crate::services::channel_bot_health::MissingGatewayEvent>,
}

fn select_latest(owners: &bson::Document, limit: usize) -> Vec<bson::Document> {
    vec![
        doc! {"$group": {"_id": "$channel_bot_id", "row": {"$first": "$$ROOT"},
        "gateway_not_received": {"$max": "$gateway_not_received"},
        "live": {"$max":{"$cond":[{"$and":[
            {"$ne":[{"$type":{"$getField":{"field":"$channel_bot_id","input":{"$literal":&owners}}}},"missing"]},
            {"$in":["$status",["pending","active"]]}
        ]},{"created_at":"$created_at","id":"$_id","row":"$$ROOT"},null]}}}},
        doc! {"$replaceWith": {"$mergeObjects": [{"$ifNull":["$live.row","$row"]}, {"gateway_not_received": "$gateway_not_received"}]}},
        doc! {"$limit": limit as i64},
    ]
}

pub(crate) async fn latest_nyxbot_rows(
    db: &mongodb::Database,
    bots: &[ChannelBot],
) -> AppResult<std::collections::HashMap<String, NyxbotHealthRow>> {
    if bots.is_empty() {
        return Ok(Default::default());
    }
    let bot_ids: Vec<_> = bots.iter().map(|bot| &bot.id).collect();
    let owners: bson::Document = bots
        .iter()
        .filter(|bot| matches!(bot.platform.as_str(), "telegram" | "telegram-new"))
        .map(|bot| {
            (
                bot.id.clone(),
                bson::Bson::Document(doc! {"owner":&bot.user_id,"version":bot.ownership_version}),
            )
        })
        .collect();
    let mut filter = doc! {"channel_bot_id":{"$in":&bot_ids}};
    if !owners.is_empty() {
        filter.insert("$expr", doc! {"$let":{"vars":{"expected":{"$getField":{
            "field":"$channel_bot_id","input":{"$literal":&owners}}}},"in":{"$or":[
                {"$eq":[{"$type":"$$expected"},"missing"]},
                {"$and":[
                    {"$eq":[{"$ifNull":["$bot_owner_id",{"$ifNull":["$org_specialist.org_id","$user_id"]}]},"$$expected.owner"]},
                    {"$eq":[{"$ifNull":["$org_specialist.org_id","$$expected.owner"]},"$$expected.owner"]},
                    {"$or":[
                        {"$eq":["$bot_ownership_version","$$expected.version"]},
                        {"$eq":[{"$ifNull":["$bot_ownership_version",null]},null]}
                    ]}
                ]}
            ]}}});
    }
    // `nyxbot_health_latest` supplies the indexed sort. Project before the
    // group so encrypted gateway material and sender data never enter the
    // aggregation; the final limit is bounded by the requested page.
    let mut pipeline = vec![
        doc! {"$match": filter},
        // Keep the sort adjacent to the indexed match. Computed legacy fields
        // before this stage prevent Mongo from using the index's ordering when
        // production's competing bot/route/status index is present.
        doc! {"$sort": {"channel_bot_id": 1, "created_at": -1, "_id": -1}},
        doc! {"$project": {
            "_id": 1, "user_id": 1, "channel_bot_id": 1, "bot_owner_id": 1, "binding_id": 1,
            "created_at": 1,
            "legacy_route_id": {"$cond":[{"$and":[
                {"$eq":[{"$ifNull":["$bot_ownership_version",null]},null]},
                {"$gt":[{"$getField":{"field":"version","input":{"$ifNull":[{"$getField":{"field":"$channel_bot_id","input":{"$literal":&owners}}},{}]}}},0_i64]}
            ]},"$route_id",null]},
            "legacy_needs_route": {"$and":[
                {"$eq":[{"$ifNull":["$bot_ownership_version",null]},null]},
                {"$gt":[{"$getField":{"field":"version","input":{"$ifNull":[{"$getField":{"field":"$channel_bot_id","input":{"$literal":&owners}}},{}]}}},0_i64]}
            ]},
            "platform": 1, "transport": 1, "status": 1, "last_error": 1, "setup_diagnostic": 1, "org_specialist": 1,
            "agent_id": 1, "org_restore_last_reason": 1,
            "org_restore_claim_id": 1, "org_restore_next_attempt_at": 1,
            "org_restore_attempts": 1,
            "gateway_not_received": {"$cond":[{"$and":[
                {"$eq":["$status","active"]},{"$eq":["$transport","gateway"]},
                {"$eq":["$delivery_status","failing"]},{"$eq":["$delivery_error","not_received"]}]},
                {"message_at":"$delivery_failed_at"},null]},
            "native_intake_refused": {"$and":[{"$eq":["$transport","gateway"]},
                {"$eq":["$status","active"]},{"$eq":["$gateway_native_phase","ready"]},
                {"$in":["$gateway_source.type",["discord","whatsapp"]]},
                {"$eq":[{"$ifNull":["$gateway_setup_release_id",null]},null]},
                {"$eq":[{"$ifNull":["$pending_agent_api_key_id",null]},null]},
                {"$eq":["$gateway_native_delivery",crate::services::channel_bot_health::codes::GATEWAY_INTAKE_REFUSED]}]},
        }},
    ];
    if bots
        .iter()
        .any(|bot| owners.contains_key(&bot.id) && bot.ownership_version > 0)
    {
        // Keep the route join out of normal/versioned histories, including mixed
        // admin pages. Each branch emits at most one row per requested bot. The
        // legacy branch must validate routes before choosing a row: a newer
        // retired legacy route must not hide an older eligible live binding.
        let mut current = vec![doc! {"$match":{"legacy_needs_route":{"$ne":true}}}];
        current.extend(select_latest(&owners, bot_ids.len()));
        let mut legacy = vec![
            doc! {"$match":{"legacy_needs_route":true}},
            doc! {"$lookup":{"from":crate::models::channel_conversation::COLLECTION_NAME,
            "localField":"legacy_route_id","foreignField":"_id","as":"legacy_route",
            "pipeline":[{"$match":{"is_active":true,"retired_by_transfer":{"$ne":true}}},
                {"$project":{"_id":1,"user_id":1,"channel_bot_id":1}}]}},
            doc! {"$match":{"$expr":{"$and":[
                {"$eq":[{"$arrayElemAt":["$legacy_route.user_id",0]},{"$ifNull":["$bot_owner_id",{"$ifNull":["$org_specialist.org_id","$user_id"]}]}]},
                {"$eq":[{"$arrayElemAt":["$legacy_route.channel_bot_id",0]},"$channel_bot_id"]}
            ]}}},
        ];
        legacy.extend(select_latest(&owners, bot_ids.len()));
        pipeline.extend([
            doc! {"$facet":{"current":current,"legacy":legacy}},
            doc! {"$project":{"rows":{"$concatArrays":["$current","$legacy"]}}},
            doc! {"$unwind":"$rows"},
            doc! {"$replaceWith":"$rows"},
            doc! {"$sort":{"channel_bot_id":1,"created_at":-1,"_id":-1}},
        ]);
    }
    pipeline.extend(select_latest(&owners, bot_ids.len()));
    pipeline.extend([
            doc! {"$project": {
                "_id": 1, "user_id": 1, "channel_bot_id": 1, "bot_owner_id": 1, "binding_id": 1, "platform": 1, "transport": 1,
                "status": 1, "last_error": 1, "setup_diagnostic": 1, "org_specialist": 1,
                "agent_id": 1, "org_restore_last_reason": 1,
                "org_restore_claim_id": 1, "org_restore_next_attempt_at": 1,
                "org_restore_attempts": 1, "native_intake_refused": 1, "gateway_not_received": 1,
            }},
            doc! {"$limit": bot_ids.len() as i64},
            crate::services::channel_bot_health::answer::event_lookup(),
            crate::services::channel_bot_health::answer::response_lookup(),
    ]);
    let collection =
        db.collection::<bson::Document>(crate::models::nyxbot_channel::COLLECTION_NAME);
    let documents: Vec<bson::Document> =
        collection.aggregate(pipeline).await?.try_collect().await?;
    let rows: Vec<NyxbotHealthRow> = documents
        .into_iter()
        .map(|document| {
            mongodb::bson::from_document(document)
                .map_err(|error| AppError::Internal(format!("invalid NyxBot health row: {error}")))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut latest = std::collections::HashMap::new();
    for row in rows {
        latest.entry(row.channel_bot_id.clone()).or_insert(row);
    }
    Ok(latest)
}

#[cfg(test)]
#[path = "nyxbot_tests.rs"]
mod tests;
