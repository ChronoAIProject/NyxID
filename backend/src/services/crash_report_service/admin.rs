use super::*;
use futures::TryStreamExt;
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportQuery {
    pub platform: Option<Platform>,
    pub app_version: Option<String>,
    pub app_build: Option<String>,
    pub kind: Option<Kind>,
    pub fingerprint: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryQuery {
    pub window: Option<String>,
    pub platform: Option<Platform>,
}
fn live() -> Document {
    doc! {"expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())}}
}
pub async fn list(
    db: &Database,
    query: &ReportQuery,
) -> AppResult<(Vec<ClientErrorReport>, Option<String>)> {
    let mut filter = live();
    if let Some(platform) = query.platform {
        filter.insert("report.platform", platform.as_str());
    }
    if let Some(kind) = query.kind {
        filter.insert("report.kind", kind.as_str());
    }
    for (key, value) in [
        ("report.app_version", &query.app_version),
        ("report.app_build", &query.app_build),
        ("fingerprint", &query.fingerprint),
    ] {
        if let Some(value) = value {
            if value.len() > 64 {
                return Err(AppError::ValidationError("Invalid report filter".into()));
            }
            filter.insert(key, value);
        }
    }
    if let Some(cursor) = &query.cursor {
        let (millis, id) = cursor
            .split_once(':')
            .ok_or_else(|| AppError::ValidationError("Invalid report cursor".into()))?;
        let millis = millis
            .parse::<i64>()
            .map_err(|_| AppError::ValidationError("Invalid report cursor".into()))?;
        uuid::Uuid::parse_str(id)
            .map_err(|_| AppError::ValidationError("Invalid report cursor".into()))?;
        let time = bson::DateTime::from_millis(millis);
        filter.insert(
            "$or",
            vec![
                doc! {"received_at": {"$lt": time}},
                doc! {"received_at": time, "_id": {"$lt": id}},
            ],
        );
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(AppError::ValidationError(
            "Report limit must be 1–100".into(),
        ));
    }
    let mut rows: Vec<ClientErrorReport> = db
        .collection(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"received_at": -1, "_id": -1})
        .limit(i64::from(limit + 1))
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let cursor = if more {
        rows.last()
            .map(|r| format!("{}:{}", r.received_at.timestamp_millis(), r.id))
    } else {
        None
    };
    Ok((rows, cursor))
}
pub async fn detail(db: &Database, id: &str) -> AppResult<ClientErrorReport> {
    uuid::Uuid::parse_str(id).map_err(|_| AppError::NotFound("Crash report not found".into()))?;
    let mut filter = live();
    filter.insert("_id", id);
    db.collection(COLLECTION_NAME)
        .find_one(filter)
        .await?
        .ok_or_else(|| AppError::NotFound("Crash report not found".into()))
}
pub struct SummaryData {
    pub versions: Vec<Document>,
    pub counters: Vec<Document>,
    pub exports: Vec<Document>,
    pub flag_enabled: bool,
}
pub async fn summary(db: &Database, query: &SummaryQuery) -> AppResult<SummaryData> {
    let hours = match query.window.as_deref().unwrap_or("24h") {
        "24h" => 24,
        "7d" => 168,
        "30d" => 720,
        _ => return Err(AppError::ValidationError("Invalid report window".into())),
    };
    let since = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(hours));
    let mut filter = live();
    filter.insert("received_at", doc! {"$gte": since});
    if let Some(platform) = query.platform {
        filter.insert("report.platform", platform.as_str());
    }
    let mut group = doc! {"_id": {"platform": "$report.platform", "environment": "$environment", "app_version": "$report.app_version", "app_build": "$report.app_build", "artifact_id": "$report.artifact_id"}, "total": {"$sum": 1}, "first_seen": {"$min": "$received_at"}, "last_seen": {"$max": "$received_at"}};
    for kind in [
        "native_crash",
        "js_fatal",
        "js_error",
        "unhandled_rejection",
        "render_error",
        "recoverable_error",
        "resource_error",
    ] {
        group.insert(
            kind,
            doc! {"$sum": {"$cond": [{"$eq": ["$report.kind", kind]}, 1, 0]}},
        );
    }
    let versions = db
        .collection::<Document>(COLLECTION_NAME)
        .aggregate(vec![
            doc! {"$match": &filter},
            doc! {"$group": group},
            doc! {"$sort": {"last_seen": -1}},
            doc! {"$limit": 200},
        ])
        .max_time(Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    let exports = db
        .collection::<Document>(COLLECTION_NAME)
        .aggregate(vec![
            doc! {"$match": filter},
            doc! {"$group": {"_id": "$export.status", "count": {"$sum": 1}}},
        ])
        .max_time(Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    let mut counter_filter = live();
    counter_filter.insert("received_at", doc! {"$gte": since});
    if let Some(platform) = query.platform {
        counter_filter.insert("platform", platform.as_str());
    }
    let counters = db
        .collection::<Document>(COUNTERS)
        .aggregate(vec![
            doc! {"$match": counter_filter},
            doc! {"$group": {"_id": "$outcome", "count": {"$sum": "$count"}}},
        ])
        .max_time(Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    let flag_enabled = !global_killed(db).await?
        && db
            .collection::<Document>(crate::models::feature_flag_override::COLLECTION_NAME)
            .find_one(doc! {"flag_key": FLAG, "enabled": true})
            .await?
            .is_some();
    Ok(SummaryData {
        versions,
        counters,
        exports,
        flag_enabled,
    })
}
