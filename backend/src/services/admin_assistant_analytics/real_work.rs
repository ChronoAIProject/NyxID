//! Weekly owner outcomes. Every read is bounded by the analytics window/deadline.
use crate::{
    errors::{AppError, AppResult},
    services::audit_service::{self, AuditActor},
};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};
use serde::{Deserialize, Serialize};

const SETTINGS: &str = "assistant_analytics_settings";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const OWNER_LIMIT: i64 = 100;
const INTERNAL_LIMIT: usize = 5000;

#[derive(Default, Serialize, Deserialize)]
pub struct InternalOwners {
    pub owner_ids: Vec<String>,
}
#[derive(Default, Serialize, Deserialize)]
pub struct OwnerWork {
    pub owner_id: String,
    pub internal: bool,
    pub completed: i64,
    pub attempted: i64,
    pub triggered: i64,
    pub helpful: i64,
    pub votes: i64,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Funnel {
    pub internal: bool,
    pub owners: i64,
    pub has_nyxbot: i64,
    pub connected_business: i64,
    pub assigned_channel_work: i64,
    pub first_real_work: i64,
    pub median_seconds_to_first_work: Option<f64>,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Week {
    pub week: String,
    pub partial: bool,
    pub merchants: i64,
    pub internal_owners: i64,
    pub completed: i64,
    pub attempted: i64,
    pub triggered: i64,
    pub helpful: i64,
    pub votes: i64,
    pub internal_completed: i64,
    pub internal_attempted: i64,
    pub internal_triggered: i64,
    pub internal_helpful: i64,
    pub internal_votes: i64,
    pub owners: Vec<OwnerWork>,
    pub owners_truncated: bool,
    pub funnel: Vec<Funnel>,
}
#[derive(Serialize)]
pub struct Weekly {
    pub threshold: u32,
    pub timezone: &'static str,
    pub weeks: Vec<Week>,
}

pub fn threshold(value: Option<u32>) -> AppResult<u32> {
    let n = value.unwrap_or(3);
    if !(1..=1000).contains(&n) {
        return Err(AppError::BadRequest(
            "Real-work threshold must be between 1 and 1000.".into(),
        ));
    }
    Ok(n)
}
pub fn week_start(at: DateTime<Utc>) -> DateTime<Utc> {
    let local = at.with_timezone(&chrono_tz::Asia::Singapore);
    let date =
        local.date_naive() - Duration::days(i64::from(local.weekday().num_days_from_monday()));
    chrono_tz::Asia::Singapore
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&Utc)
}
fn week_expr(date: &str) -> Document {
    doc! {"$dateToString":{"date":{"$dateTrunc":{"date":date,"unit":"week","startOfWeek":"monday","timezone":"Asia/Singapore"}},"format":"%Y-%m-%d","timezone":"Asia/Singapore"}}
}
fn range(from: DateTime<Utc>, to: DateTime<Utc>) -> Document {
    doc! {"$gte":bson::DateTime::from_chrono(from),"$lt":bson::DateTime::from_chrono(to)}
}
fn timeout(error: mongodb::error::Error) -> AppError {
    if matches!(error.kind.as_ref(),mongodb::error::ErrorKind::Command(e) if e.code==50) {
        AppError::AdminUsageQueryTimeout
    } else {
        error.into()
    }
}
async fn query<T: serde::de::DeserializeOwned>(
    db: &Database,
    collection: &str,
    pipeline: Vec<Document>,
) -> AppResult<Vec<T>> {
    let rows: Vec<Document> = db
        .collection::<Document>(collection)
        .aggregate(pipeline)
        .max_time(TIMEOUT)
        .await
        .map_err(timeout)?
        .try_collect()
        .await
        .map_err(timeout)?;
    rows.into_iter()
        .map(|r| {
            bson::from_document(r)
                .map_err(|_| AppError::Internal("Invalid analytics projection".into()))
        })
        .collect()
}
pub async fn internal(db: &Database) -> AppResult<InternalOwners> {
    Ok(db
        .collection::<InternalOwners>(SETTINGS)
        .find_one(doc! {"_id":"internal_owners"})
        .max_time(TIMEOUT)
        .await
        .map_err(timeout)?
        .unwrap_or_default())
}
pub async fn set_internal(
    db: &Database,
    actor: &AuditActor,
    mut input: InternalOwners,
) -> AppResult<InternalOwners> {
    if input.owner_ids.len() > INTERNAL_LIMIT
        || input
            .owner_ids
            .iter()
            .any(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err(AppError::ValidationError(
            "Provide at most 5000 owner UUIDs.".into(),
        ));
    }
    input.owner_ids.sort();
    input.owner_ids.dedup();
    let existing = db
        .collection::<Document>("users")
        .count_documents(
            doc! {"_id":{"$in":&input.owner_ids},"user_type":{"$in":["person","org",null]}},
        )
        .max_time(TIMEOUT)
        .await
        .map_err(timeout)?;
    if existing != input.owner_ids.len() as u64 {
        return Err(AppError::ValidationError(
            "Every internal owner must be a person or organization.".into(),
        ));
    }
    let db = db.clone();
    let actor = actor.clone();
    let owner_ids = input.owner_ids.clone();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .and_run2(async move |tx| {
            db.collection::<Document>(SETTINGS)
                .update_one(
                    doc! {"_id":"internal_owners"},
                    doc! {"$set":{"owner_ids":&owner_ids,"updated_at":bson::DateTime::now()}},
                )
                .upsert(true)
                .session(&mut *tx)
                .await?;
            let result = audit_service::log_actor_event_in_transaction(
                &db,
                tx,
                &actor,
                "assistant_internal_owners_updated",
                serde_json::json!({"owner_ids":owner_ids}),
            )
            .await;
            super::super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)?;
    Ok(input)
}

/// Run turns are excluded; their terminal run row is the sole counting identity.
/// Voice copies retain the original settlement timestamp, so a delayed copy or
/// cancellation changes only that turn in its original week.
fn outcomes(from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Document> {
    vec![
        doc! {"$match":{"turn_metadata.work.owner_id":{"$exists":true},"role":"assistant","turn_metadata.work.settled_at":range(from,to),"turn_metadata.work.business_system_success":true,"turn_metadata.work.trigger_run_id":null}},
        doc! {"$project":{"turn_id":1,"created_at":1,"work":"$turn_metadata.work","completed":{"$eq":["$status","completed"]}}},
        doc! {"$group":{"_id":"$turn_id","turn_id":{"$first":"$turn_id"},"at":{"$min":"$work.settled_at"},"owner_id":{"$first":"$work.owner_id"},"source":{"$first":"$work.source"},"completed":{"$min":"$completed"}}},
        doc! {"$unionWith":{"coll":"trigger_runs","pipeline":[
            {"$match":{"work.owner_id":{"$exists":true},"finished_at":range(from,to),"outcome":{"$in":["completed","failed","skipped"]},"work.business_system_success":true}},
            {"$project":{"_id":1,"turn_id":1,"at":"$finished_at","owner_id":"$work.owner_id","source":"$work.source","completed":{"$eq":["$outcome","completed"]}}}
        ]}},
    ]
}

pub async fn weekly(
    db: &Database,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    threshold: u32,
) -> AppResult<Weekly> {
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        weekly_inner(db, from, to, threshold),
    )
    .await
    .map_err(|_| AppError::AdminUsageQueryTimeout)?
}
async fn weekly_inner(
    db: &Database,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    threshold: u32,
) -> AppResult<Weekly> {
    let internal = internal(db).await?.owner_ids;
    let mut pipeline = outcomes(from, to);
    pipeline.extend([
        doc!{"$lookup":{"from":"assistant_message_feedback","localField":"turn_id","foreignField":"turn_id","pipeline":[{"$sort":{"updated_at":-1}},{"$group":{"_id":"$user_id","vote":{"$first":"$vote"}}},{"$group":{"_id":null,"votes":{"$sum":1},"helpful":{"$sum":{"$cond":[{"$eq":["$vote","up"]},1,0]}}}}],"as":"feedback"}},
        doc!{"$set":{"week":week_expr("$at"),"internal":{"$in":["$owner_id",&internal]},"triggered":{"$in":["$source",["scheduled","event"]]}}},
        doc!{"$group":{"_id":{"week":"$week","owner":"$owner_id"},"internal":{"$first":"$internal"},"completed":{"$sum":{"$cond":["$completed",1,0]}},"attempted":{"$sum":1},"triggered":{"$sum":{"$cond":[{"$and":["$completed","$triggered"]},1,0]}},"votes":{"$sum":{"$cond":["$completed",{"$ifNull":[{"$first":"$feedback.votes"},0]},0]}},"helpful":{"$sum":{"$cond":["$completed",{"$ifNull":[{"$first":"$feedback.helpful"},0]},0]}}}},
        doc!{"$sort":{"_id.week":1,"completed":-1,"_id.owner":1}},
        doc!{"$group":{"_id":"$_id.week",
            "merchants":{"$sum":{"$cond":[{"$and":[{"$not":["$internal"]},{"$gte":["$completed",i64::from(threshold)]}]},1,0]}},
            "internal_owners":{"$sum":{"$cond":[{"$and":["$internal",{"$gte":["$completed",i64::from(threshold)]}]},1,0]}},
            "completed":{"$sum":{"$cond":["$internal",0,"$completed"]}},"attempted":{"$sum":{"$cond":["$internal",0,"$attempted"]}},"triggered":{"$sum":{"$cond":["$internal",0,"$triggered"]}},"votes":{"$sum":{"$cond":["$internal",0,"$votes"]}},"helpful":{"$sum":{"$cond":["$internal",0,"$helpful"]}},
            "internal_completed":{"$sum":{"$cond":["$internal","$completed",0]}},"internal_attempted":{"$sum":{"$cond":["$internal","$attempted",0]}},"internal_triggered":{"$sum":{"$cond":["$internal","$triggered",0]}},"internal_votes":{"$sum":{"$cond":["$internal","$votes",0]}},"internal_helpful":{"$sum":{"$cond":["$internal","$helpful",0]}},
            "owners_count":{"$sum":1},
            "owners":{"$firstN":{"n":OWNER_LIMIT,"input":{"owner_id":"$_id.owner","internal":"$internal","completed":"$completed","attempted":"$attempted","triggered":"$triggered","votes":"$votes","helpful":"$helpful"}}}
        }},
        doc!{"$set":{"week":"$_id","partial":false,"funnel":[],"owners_truncated":{"$gt":["$owners_count",OWNER_LIMIT]}}},
    ]);
    let (rows, cohorts) = tokio::try_join!(
        query::<Week>(db, "assistant_messages", pipeline),
        funnel(db, from, to, &internal)
    )?;
    let mut rows: std::collections::HashMap<_, _> = rows
        .into_iter()
        .map(|row| (row.week.clone(), row))
        .collect();
    let mut weeks = Vec::new();
    let mut at = week_start(from);
    while at < to {
        let day = at
            .with_timezone(&chrono_tz::Asia::Singapore)
            .format("%Y-%m-%d")
            .to_string();
        let mut week = rows.remove(&day).unwrap_or_else(|| Week {
            week: day.clone(),
            ..Default::default()
        });
        week.partial = at < from || at + Duration::weeks(1) > to;
        week.funnel = cohorts
            .iter()
            .filter(|r| r.week == day)
            .map(|r| Funnel {
                internal: r.funnel.internal,
                owners: r.funnel.owners,
                has_nyxbot: r.funnel.has_nyxbot,
                connected_business: r.funnel.connected_business,
                assigned_channel_work: r.funnel.assigned_channel_work,
                first_real_work: r.funnel.first_real_work,
                median_seconds_to_first_work: r.funnel.median_seconds_to_first_work,
            })
            .collect();
        weeks.push(week);
        at += Duration::weeks(1);
    }
    Ok(Weekly {
        threshold,
        timezone: "Asia/Singapore",
        weeks,
    })
}

#[derive(Deserialize)]
struct Cohort {
    week: String,
    #[serde(flatten)]
    funnel: Funnel,
}

/// Cohorts are clipped to the requested creation window and observed through `to`.
/// All joins are owner-scoped and time-bounded; no transcript or credential is read.
async fn funnel(
    db: &Database,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    internal: &[String],
) -> AppResult<Vec<Cohort>> {
    let end = bson::DateTime::from_chrono(to);
    query(db,"users",vec![
        doc!{"$match":{"created_at":range(from,to),"user_type":{"$in":["person","org",null]}}},
        doc!{"$project":{"created_at":1,"user_type":1}},
        doc!{"$lookup":{"from":"assistant_agents","localField":"_id","foreignField":"user_id","let":{"org":{"$eq":["$user_type","org"]}},"pipeline":[
            {"$match":{"created_at":{"$lt":end},"$expr":{"$or":["$$org",{"$eq":["$kind","nyxbot"]}]}}},
            {"$limit":1},{"$project":{"_id":1}}
        ],"as":"agent"}},
        doc!{"$lookup":{"from":"user_services","localField":"_id","foreignField":"user_id","pipeline":[
            {"$match":{"created_at":{"$lt":end},"deleted_at":null,"credential_binding":{"$ne":"platform"},
                "$or":[{"source":{"$ne":"auto_provision"}},{"api_key_id":{"$type":"string"}},{"node_id":{"$type":"string","$ne":""}}],
                "slug":{"$not":{"$regex":"^llm-"}}}},
            {"$project":{"catalog_service_id":1}},
            {"$lookup":{"from":"downstream_services","localField":"catalog_service_id","foreignField":"_id","pipeline":[{"$project":{"service_category":1,"inference":1,"slug":1}}],"as":"catalog"}},
            {"$match":{"$or":[{"catalog_service_id":null},{"catalog":{"$elemMatch":{"service_category":"connection","inference":null,"slug":{"$not":{"$regex":"^llm-"}}}}}]}},
            {"$limit":1},{"$project":{"_id":1}}
        ],"as":"connection"}},
        doc!{"$lookup":{"from":"assistant_messages","localField":"_id","foreignField":"turn_metadata.work.owner_id","pipeline":[
            {"$match":{"turn_metadata.work.owner_id":{"$exists":true},"created_at":range(from,to),"role":"user","turn_metadata.work.source":"channel"}},
            {"$limit":1},{"$project":{"_id":1}}
        ],"as":"channel"}},
        doc!{"$lookup":{"from":"assistant_messages","localField":"_id","foreignField":"turn_metadata.work.owner_id","pipeline":[
            {"$match":{"turn_metadata.work.owner_id":{"$exists":true},"turn_metadata.work.settled_at":range(from,to),"role":"assistant","turn_metadata.work.business_system_success":true,"turn_metadata.work.trigger_run_id":null}},
            {"$project":{"turn_id":1,"at":"$turn_metadata.work.settled_at","completed":{"$eq":["$status","completed"]}}},
            {"$group":{"_id":"$turn_id","at":{"$min":"$at"},"completed":{"$min":"$completed"}}},
            {"$match":{"completed":true}},
            {"$sort":{"at":1}},{"$limit":1},{"$project":{"at":1}}
        ],"as":"first_turn"}},
        doc!{"$lookup":{"from":"trigger_runs","localField":"_id","foreignField":"work.owner_id","pipeline":[
            {"$match":{"work.owner_id":{"$exists":true},"finished_at":range(from,to),"outcome":"completed","work.business_system_success":true}},
            {"$sort":{"finished_at":1}},{"$limit":1},{"$project":{"at":"$finished_at"}}
        ],"as":"first_run"}},
        doc!{"$set":{"has_nyxbot":{"$gt":[{"$size":"$agent"},0]},"connected":{"$gt":[{"$size":"$connection"},0]},"assigned":{"$gt":[{"$size":"$channel"},0]},
            "first":{"$min":{"$concatArrays":["$first_turn.at","$first_run.at"]}}}},
        doc!{"$set":{"connected":{"$and":["$has_nyxbot","$connected"]},"assigned":{"$and":["$has_nyxbot","$connected","$assigned"]},
            "activated":{"$and":["$has_nyxbot","$connected","$assigned",{"$ne":["$first",null]},{"$gte":["$first","$created_at"]}]}}},
        doc!{"$group":{"_id":{"week":week_expr("$created_at"),"internal":{"$in":["$_id",internal]}},
            "owners":{"$sum":1},"has_nyxbot":{"$sum":{"$cond":["$has_nyxbot",1,0]}},"connected_business":{"$sum":{"$cond":["$connected",1,0]}},"assigned_channel_work":{"$sum":{"$cond":["$assigned",1,0]}},"first_real_work":{"$sum":{"$cond":["$activated",1,0]}},
            "median_seconds_to_first_work":{"$median":{"input":{"$cond":["$activated",{"$divide":[{"$subtract":["$first","$created_at"]},1000]},null]},"method":"approximate"}}}},
        doc!{"$set":{"week":"$_id.week","internal":"$_id.internal"}},
    ]).await
}

/// Startup-only, additive indexes. The owner/time prefixes support funnel joins;
/// signal/time supports the weekly report without scanning ordinary chat history.
pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    for (collection, name, keys) in [
        (
            "assistant_messages",
            "assistant_work_signal_time",
            doc! {"turn_metadata.work.business_system_success":1,"turn_metadata.work.settled_at":1},
        ),
        (
            "assistant_messages",
            "assistant_work_owner_role_time",
            doc! {"turn_metadata.work.owner_id":1,"role":1,"created_at":1},
        ),
        (
            "assistant_messages",
            "assistant_work_owner_role_settlement",
            doc! {"turn_metadata.work.owner_id":1,"role":1,"turn_metadata.work.settled_at":1},
        ),
        (
            "trigger_runs",
            "trigger_work_signal_time",
            doc! {"work.business_system_success":1,"finished_at":1},
        ),
        (
            "trigger_runs",
            "trigger_work_owner_time",
            doc! {"work.owner_id":1,"finished_at":1},
        ),
        (
            "assistant_message_feedback",
            "feedback_work_turn",
            doc! {"turn_id":1,"user_id":1},
        ),
        (
            "users",
            "assistant_cohort_creation",
            doc! {"created_at":1,"user_type":1},
        ),
        (
            "assistant_agents",
            "assistant_cohort_owner_creation",
            doc! {"user_id":1,"created_at":1},
        ),
        (
            "user_services",
            "assistant_cohort_connection_creation",
            doc! {"user_id":1,"created_at":1},
        ),
    ] {
        let partial_filter = match collection {
            "assistant_messages" => Some(doc! {"turn_metadata.work.owner_id": {"$exists": true}}),
            "trigger_runs" => Some(doc! {"work.owner_id": {"$exists": true}}),
            _ => None,
        };
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .partial_filter_expression(partial_filter)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
