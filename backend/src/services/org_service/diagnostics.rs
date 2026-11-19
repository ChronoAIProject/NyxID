//! Read-only signal for deletions that require an administrator to retry.
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{Database, IndexModel, options::IndexOptions};

use crate::{
    errors::AppResult,
    models::startup_diagnostic::{DiagnosticSeverity, StartupDiagnostic},
};

const INDEX: &str = "org_deletion_requested";
const STALE_SECONDS: i64 = 15 * 60;
const COUNT_LIMIT: i64 = 1_000;
const SAMPLE_LIMIT: usize = 5;

fn marked() -> Document {
    doc! {"user_type":"org","deletion_audit.requested_at":{"$type":"date"}}
}
pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>("users")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"deletion_audit.requested_at":1,"_id":1})
                .options(
                    IndexOptions::builder()
                        .name(INDEX.to_owned())
                        .partial_filter_expression(marked())
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub(crate) async fn diagnostic(db: &Database) -> AppResult<Option<StartupDiagnostic>> {
    // Oldest first: if the bounded prefix includes a recent marker, no older
    // omitted marker can qualify. The extra row distinguishes an exact count
    // from a lower bound. Only ids and timestamps leave MongoDB.
    let rows: Vec<Document> = db.collection::<Document>("users").aggregate(vec![
        doc! {"$match":marked()},
        doc! {"$sort":{"deletion_audit.requested_at":1,"_id":1}},
        doc! {"$limit":COUNT_LIMIT + 1},
        doc! {"$project":{"_id":1,"requested_at":"$deletion_audit.requested_at","now":"$$NOW",
            "age_seconds":{"$floor":{"$divide":[{"$subtract":["$$NOW","$deletion_audit.requested_at"]},1000]}}}},
        doc! {"$match":{"age_seconds":{"$gte":STALE_SECONDS}}},
        doc! {"$group":{"_id":null,"count":{"$sum":1},"oldest":{"$first":"$requested_at"},
            "now":{"$first":"$now"},"ids":{"$push":"$_id"}}},
        doc! {"$project":{"count":1,"oldest":1,"now":1,"ids":{"$slice":["$ids",SAMPLE_LIMIT as i64]}}},
    ]).max_time(std::time::Duration::from_secs(2))
        .await?.try_collect().await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let count = row
        .get_i32("count")
        .map(i64::from)
        .or_else(|_| row.get_i64("count"))
        .map_err(|_| {
            crate::errors::AppError::Internal("Organization deletion count invalid".into())
        })?;
    let oldest = row
        .get_datetime("oldest")
        .map_err(|_| {
            crate::errors::AppError::Internal("Organization deletion timestamp invalid".into())
        })?
        .to_chrono();
    let now = row
        .get_datetime("now")
        .map_err(|_| {
            crate::errors::AppError::Internal("Organization deletion timestamp invalid".into())
        })?
        .to_chrono();
    let ids: Vec<_> = row
        .get_array("ids")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .filter_map(|id| uuid::Uuid::parse_str(id).ok())
        .map(|id| id.to_string())
        .collect();
    let count_text = if count > COUNT_LIMIT {
        format!("at least {COUNT_LIMIT}")
    } else {
        count.to_string()
    };
    let code = "organization_deletion_incomplete";
    Ok(Some(StartupDiagnostic {
        id: code.into(), active:true, code:code.into(),
        severity: DiagnosticSeverity::Action, technical_details: None,
        summary: format!("Organization deletions awaiting retry: {count_text}"),
        detail: format!("Requested at least {STALE_SECONDS} seconds ago; oldest pending age: {} seconds. Sample organization ids: {}.", (now-oldest).num_seconds(), ids.join(", ")),
        remediation: "An active organization administrator should retry DELETE /orgs/{id}. Deletion does not resume automatically; the marker does not grant access.".into(),
        detected_at: oldest, updated_at: now, resolved_at: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    tokio::task_local! { static READ: (); }
    #[tokio::test]
    async fn incomplete_deletions_are_visible_with_one_indexed_read_and_database_time() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let commands = seen.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "org_deletion_diagnostic",
            EventHandler::callback(move |event| {
                if READ.try_with(|_| ()).is_ok()
                    && let CommandEvent::Started(e) = event
                {
                    commands.lock().unwrap().push((e.command_name, e.command));
                }
            }),
        )
        .await;
        ensure_indexes(&db).await.unwrap();
        assert!(READ.scope((), diagnostic(&db)).await.unwrap().is_none());
        assert_eq!(seen.lock().unwrap().len(), 1);
        let old = uuid::Uuid::new_v4().to_string();
        let recent = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>("users").insert_many([
            doc!{"_id":&old,"user_type":"org","deletion_audit":{"requested_at":bson::DateTime::from_millis(0)}},
            doc!{"_id":&recent,"user_type":"org"},
            doc!{"_id":uuid::Uuid::new_v4().to_string(),"user_type":"person","deletion_audit":{"requested_at":bson::DateTime::from_millis(0)}},
            doc!{"_id":uuid::Uuid::new_v4().to_string(),"user_type":"org"},
        ]).await.unwrap();
        db.collection::<Document>("users")
            .update_one(
                doc! {"_id":&recent},
                doc! {"$currentDate":{"deletion_audit.requested_at":true}},
            )
            .await
            .unwrap();
        let before: Vec<Document> = db
            .collection("users")
            .find(doc! {})
            .sort(doc! {"_id":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        seen.lock().unwrap().clear();
        let item = READ.scope((), diagnostic(&db)).await.unwrap().unwrap();
        assert_eq!(item.summary, "Organization deletions awaiting retry: 1");
        assert!(item.detail.contains(&old));
        assert!(!item.detail.contains(&recent));
        assert_eq!(item.detected_at.timestamp(), 0);
        let after: Vec<Document> = db
            .collection("users")
            .find(doc! {})
            .sort(doc! {"_id":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(before, after);
        let commands = seen.lock().unwrap().clone();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, "aggregate");
        println!("DELETION_DIAGNOSTIC_COMMANDS 1 aggregate, 0 writes");
        let mut command = commands[0].1.clone();
        for field in ["$db", "lsid", "$clusterTime", "readConcern"] {
            command.remove(field);
        }
        let explain = db
            .run_command(doc! {"explain":command,"verbosity":"queryPlanner"})
            .await
            .unwrap();
        assert!(explain.to_string().contains(INDEX), "{explain:?}");
        db.collection::<Document>("users")
            .delete_one(doc! {"_id":&old})
            .await
            .unwrap();
        assert!(diagnostic(&db).await.unwrap().is_none());
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn deletion_diagnostic_bounds_count_and_samples() {
        let db =
            crate::test_utils::connect_transaction_test_database("org_deletion_diagnostic_bound")
                .await;
        ensure_indexes(&db).await.unwrap();
        db.collection::<Document>("users")
            .insert_many((0..1002).map(|_| {
                doc! {
                    "_id":uuid::Uuid::new_v4().to_string(),"user_type":"org",
                    "deletion_audit":{"requested_at":bson::DateTime::from_millis(0)}
                }
            }))
            .await
            .unwrap();
        let item = diagnostic(&db).await.unwrap().unwrap();
        assert_eq!(
            item.summary,
            "Organization deletions awaiting retry: at least 1000"
        );
        let samples = item
            .detail
            .split("Sample organization ids: ")
            .nth(1)
            .unwrap()
            .trim_end_matches('.');
        assert_eq!(samples.split(", ").count(), 5);
        for id in samples.split(", ") {
            uuid::Uuid::parse_str(id).unwrap();
        }
        db.drop().await.unwrap();
    }
}
