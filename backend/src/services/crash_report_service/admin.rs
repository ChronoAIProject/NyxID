use super::*;
use crate::models::client_release_registry::{COLLECTION_NAME as RELEASES, ClientRelease};
use futures::TryStreamExt;
use serde::Deserialize;
use std::collections::HashSet;

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
#[derive(Clone, Debug)]
pub struct ReportData {
    pub row: ClientErrorReport,
    pub release_verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ReleaseIdentity {
    platform: String,
    app_version: String,
    app_build: String,
    artifact_id: Option<String>,
}

fn release_identity(row: &ClientErrorReport) -> ReleaseIdentity {
    let report = &row.report;
    ReleaseIdentity {
        platform: report.platform.as_str().to_owned(),
        app_version: report.app_version.clone(),
        app_build: report.app_build.clone(),
        artifact_id: (report.platform == Platform::Web)
            .then(|| report.artifact_id.clone())
            .flatten(),
    }
}

async fn verified_release_identities(
    db: &Database,
    rows: &[ClientErrorReport],
) -> AppResult<HashSet<ReleaseIdentity>> {
    if rows.is_empty() {
        return Ok(HashSet::new());
    }
    let mut identities = HashSet::new();
    let mut filters = Vec::new();
    for row in rows {
        let identity = release_identity(row);
        if identities.insert(identity.clone()) {
            let mut filter = doc! {
                "platform": &identity.platform,
                "app_version": &identity.app_version,
                "app_build": &identity.app_build,
            };
            if identity.platform == Platform::Web.as_str() {
                filter.insert(
                    "artifact_id",
                    identity
                        .artifact_id
                        .as_deref()
                        .map(|artifact| bson::Bson::String(artifact.to_owned()))
                        .unwrap_or(bson::Bson::Null),
                );
            }
            filters.push(filter);
        }
    }
    let mut verified = HashSet::new();
    let mut cursor = db
        .collection::<ClientRelease>(RELEASES)
        .find(doc! {"$or": filters})
        .await?;
    while let Some(release) = cursor.try_next().await? {
        verified.insert(ReleaseIdentity {
            platform: release.platform.as_str().to_owned(),
            app_version: release.app_version,
            app_build: release.app_build,
            artifact_id: (release.platform == Platform::Web)
                .then_some(release.artifact_id)
                .flatten(),
        });
    }
    Ok(verified)
}

pub async fn list(
    db: &Database,
    query: &ReportQuery,
) -> AppResult<(Vec<ReportData>, Option<String>)> {
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
    let verified = verified_release_identities(db, &rows).await?;
    Ok((
        rows.into_iter()
            .map(|row| ReportData {
                release_verified: verified.contains(&release_identity(&row)),
                row,
            })
            .collect(),
        cursor,
    ))
}
pub async fn detail(db: &Database, id: &str) -> AppResult<ReportData> {
    uuid::Uuid::parse_str(id).map_err(|_| AppError::NotFound("Crash report not found".into()))?;
    let mut filter = live();
    filter.insert("_id", id);
    let row = db
        .collection::<ClientErrorReport>(COLLECTION_NAME)
        .find_one(filter)
        .await?
        .ok_or_else(|| AppError::NotFound("Crash report not found".into()))?;
    let verified = verified_release_identities(db, std::slice::from_ref(&row)).await?;
    Ok(ReportData {
        release_verified: verified.contains(&release_identity(&row)),
        row,
    })
}
#[derive(Clone, Debug)]
pub struct VersionData {
    pub platform: String,
    pub environment: String,
    pub app_version: String,
    pub app_build: String,
    pub artifact_ids: Vec<String>,
    pub counts: std::collections::BTreeMap<String, i64>,
    pub total: i64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rollout {
    Off,
    Killed,
    Global,
    Cohort,
}
pub struct SummaryData {
    pub versions: Vec<VersionData>,
    pub unverified_releases: Vec<VersionData>,
    pub counters: std::collections::BTreeMap<String, i64>,
    pub exports: std::collections::BTreeMap<String, i64>,
    pub rollout: Rollout,
}
fn number(d: &Document, key: &str) -> i64 {
    d.get_i64(key)
        .unwrap_or_else(|_| i64::from(d.get_i32(key).unwrap_or(0)))
}
fn parse_versions(rows: Vec<Document>) -> AppResult<Vec<VersionData>> {
    rows.into_iter()
        .map(|d| {
            let invalid = || AppError::Internal("Invalid crash summary projection".into());
            let id = d.get_document("_id").map_err(|_| invalid())?;
            Ok(VersionData {
                platform: id.get_str("platform").map_err(|_| invalid())?.into(),
                environment: id.get_str("environment").map_err(|_| invalid())?.into(),
                app_version: id.get_str("app_version").map_err(|_| invalid())?.into(),
                app_build: id.get_str("app_build").map_err(|_| invalid())?.into(),
                artifact_ids: d
                    .get_array("artifact_ids")
                    .map_err(|_| invalid())?
                    .iter()
                    .filter_map(bson::Bson::as_str)
                    .map(str::to_owned)
                    .collect(),
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
                .map(|k| (k.into(), number(&d, k)))
                .collect(),
                total: number(&d, "total"),
                first_seen: d
                    .get_datetime("first_seen")
                    .map_err(|_| invalid())?
                    .to_chrono(),
                last_seen: d
                    .get_datetime("last_seen")
                    .map_err(|_| invalid())?
                    .to_chrono(),
            })
        })
        .collect()
}
fn parse_counts(rows: Vec<Document>) -> std::collections::BTreeMap<String, i64> {
    rows.into_iter()
        .map(|d| {
            (
                d.get_str("_id").unwrap_or("unknown").to_owned(),
                number(&d, "count"),
            )
        })
        .collect()
}
pub async fn rollout(db: &Database) -> AppResult<Rollout> {
    let rows: Vec<crate::models::feature_flag_override::FeatureFlagOverride> = db
        .collection(crate::models::feature_flag_override::COLLECTION_NAME)
        .find(doc! {"flag_key": FLAG,"definition_id":null})
        .await?
        .try_collect()
        .await?;
    let global = rows.iter().find(|r| {
        r.org_user_id.is_none()
            && r.target_kind == crate::models::feature_flag_override::FlagTargetKind::Global
            && r.target_key.is_none()
    });
    Ok(match global {
        Some(r) if !r.enabled => Rollout::Killed,
        Some(_) => Rollout::Global,
        None if rows.iter().any(|r| r.enabled) => Rollout::Cohort,
        _ => Rollout::Off,
    })
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
    let mut group = doc! {
        "_id": {
            "platform": "$report.platform", "environment": "$environment",
            "app_version": "$report.app_version", "app_build": "$report.app_build"
        },
        "artifact_ids": {"$addToSet": "$report.artifact_id"},
        "total": {"$sum": 1},
        "first_seen": {"$min": "$received_at"}, "last_seen": {"$max": "$received_at"}
    };
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
    let mut version_pipeline = vec![
        doc! {"$match": &filter},
        super::releases::verification_lookup(),
        doc! {"$match": {"registered_release.0": {"$exists": true}}},
        doc! {"$group": group},
        doc! {"$sort": {"last_seen": -1}},
        doc! {"$limit": 200},
        doc! {
            "$set": {
                "artifact_ids": {
                    "$slice": [{
                        "$sortArray": {
                            "input": {
                                "$filter": {
                                    "input": "$artifact_ids",
                                    "as": "artifact",
                                    "cond": {"$ne": ["$$artifact", null]}
                                }
                            },
                            "sortBy": 1
                        }
                    }, 10]
                }
            }
        },
    ];
    let versions: Vec<Document> = db
        .collection::<Document>(COLLECTION_NAME)
        .aggregate(version_pipeline.clone())
        .max_time(Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    version_pipeline[2] = doc! {"$match": {"registered_release.0": {"$exists": false}}};
    let unverified: Vec<Document> = db
        .collection::<Document>(COLLECTION_NAME)
        .aggregate(version_pipeline)
        .max_time(Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    let exports: Vec<Document> = db
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
    let counters: Vec<Document> = db
        .collection::<Document>(COUNTERS)
        .aggregate(vec![
            doc! {"$match": counter_filter},
            doc! {"$group": {"_id": "$outcome", "count": {"$sum": "$count"}}},
        ])
        .max_time(Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    Ok(SummaryData {
        versions: parse_versions(versions)?,
        unverified_releases: parse_versions(unverified)?,
        counters: parse_counts(counters),
        exports: parse_counts(exports),
        rollout: rollout(db).await?,
    })
}
