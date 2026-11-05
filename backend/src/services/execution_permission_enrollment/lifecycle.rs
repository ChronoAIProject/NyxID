//! Best-effort hint reads must never poison the transaction for a user action.
//! Only hint/repair writes join that transaction. Permission authority is separate.
use super::*;
use crate::models::execution_enrollment_repair::{
    COLLECTION_NAME as REPAIRS, ExecutionEnrollmentRepair as Repair,
};
use chrono::{Duration, Utc};
use mongodb::options::{IndexOptions, WriteConcern};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(crate) enum Change<'a> {
    Membership { org: &'a str, role: Option<OrgRole> },
    Account { active: bool },
    Current,
}

struct Hint {
    enabled: bool,
    orgs: Vec<String>,
}

// Test-only delay is scoped to a unique fixture DB, never a global Mongo failpoint.
#[cfg(test)]
pub(super) static DELAY_READ: std::sync::LazyLock<dashmap::DashSet<String>> =
    std::sync::LazyLock::new(dashmap::DashSet::new);

async fn read_hint(db: &Database, actor: &str, change: Change<'_>) -> AppResult<Option<Hint>> {
    #[cfg(test)]
    if DELAY_READ.contains(db.name()) {
        tokio::time::sleep(READ_TIMEOUT * 2).await;
    }
    let person = db
        .collection::<Document>(USERS)
        .find_one(doc! {"_id":actor})
        .projection(doc! {"user_type":1,"is_active":1,"platform_personal_deleting":1})
        .max_time(READ_TIMEOUT)
        .await?;
    let active = person.as_ref().is_some_and(|person| {
        person.get_str("user_type").ok() == Some("person")
            && !person
                .get_bool("platform_personal_deleting")
                .unwrap_or(false)
            && match change {
                Change::Account { active } => active,
                _ => person.get_bool("is_active").unwrap_or(false),
            }
    });
    if !active {
        return Ok(Some(Hint {
            enabled: false,
            orgs: Vec::new(),
        }));
    }
    let Some(mut scope) = memberships(db, actor, None).await? else {
        return Ok(None);
    };
    // The standalone reads cannot see the caller's uncommitted membership write.
    if let Change::Membership { org, role } = change {
        scope.retain(|(id, _)| id != org);
        if let Some(role) = role {
            scope.push((org.to_owned(), role));
        }
    }
    if scope.len() > MAX_MEMBERSHIPS {
        return Ok(None);
    }
    let Some(rows) = rows(db, actor, &scope, None).await? else {
        return Ok(None);
    };
    Ok(Some(Hint {
        enabled: resolve(&rows, &scope, actor, false),
        orgs: scope.into_iter().map(|(id, _)| id).collect(),
    }))
}

async fn bounded_hint(db: &Database, actor: &str, change: Change<'_>) -> Option<Hint> {
    // A server maxTimeMS error can abort a Mongo transaction. These queries are
    // deliberately standalone, with a whole-read deadline as well as query limits.
    tokio::time::timeout(READ_TIMEOUT, read_hint(db, actor, change))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
}

async fn write_hint(
    db: &Database,
    actor: &str,
    enabled: bool,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let update = if enabled {
        doc! {"$set":{"execution_permission_enrollment":1_i32}}
    } else {
        doc! {"$unset":{"execution_permission_enrollment":""}}
    };
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":actor}, update)
        .session(tx)
        .await?;
    Ok(())
}

pub(crate) async fn apply(
    db: &Database,
    actor: &str,
    change: Change<'_>,
    tx: &mut ClientSession,
) -> AppResult<()> {
    // Disabling needs no hint lookup at all. Reactivation reads current flags,
    // substituting the account state already validated by the action's writer.
    let hint = if matches!(change, Change::Account { active: false }) {
        Some(Hint {
            enabled: false,
            orgs: Vec::new(),
        })
    } else {
        bounded_hint(db, actor, change).await
    };
    if let Some(hint) = hint {
        write_hint(db, actor, hint.enabled, tx).await?;
        db.collection::<Repair>(REPAIRS)
            .delete_one(doc! {"actor_id":actor})
            .session(tx)
            .await?;
    } else {
        if let Change::Membership { org, .. } = change {
            jobs::enqueue(db, org, tx).await?;
        }
        defer_person(db, actor, tx).await?;
    }
    Ok(())
}

/// Retain the actor after membership deletion, or when a failed account-state
/// lookup cannot identify its orgs yet. This write commits with the user action.
pub(super) async fn defer_person(
    db: &Database,
    actor: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Repair>(REPAIRS).update_one(doc! {"actor_id":actor}, doc! {
        "$set":{"generation":Uuid::new_v4().to_string(),"retry_after":bson::DateTime::now()},
        "$setOnInsert":{"_id":Uuid::new_v4().to_string(),"actor_id":actor}
    }).upsert(true).session(tx).await?;
    Ok(())
}

pub(super) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    for (keys, name, unique) in [
        (
            doc! {"actor_id":1},
            "execution_enrollment_repair_actor",
            true,
        ),
        (
            doc! {"retry_after":1,"_id":1},
            "execution_enrollment_repair_due",
            false,
        ),
    ] {
        db.collection::<Repair>(REPAIRS)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .unique(unique)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    Ok(())
}

/// Only explicitly deferred people are visited, in bounded batches. No population scan.
pub(super) async fn repair_due(db: &Database) -> AppResult<()> {
    let pending: Vec<Repair> = db
        .collection::<Repair>(REPAIRS)
        .find(doc! {"retry_after":{"$lte":bson::DateTime::now()}})
        .sort(doc! {"retry_after":1,"_id":1})
        .limit(25)
        .max_time(READ_TIMEOUT)
        .await?
        .try_collect()
        .await?;
    for repair in pending {
        let filter = doc! {"_id":&repair.id,"generation":&repair.generation};
        let Some(hint) = bounded_hint(db, &repair.actor_id, Change::Current).await else {
            db.collection::<Repair>(REPAIRS).update_one(filter, doc! {
                "$set":{"retry_after":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(5))}
            }).await?;
            continue;
        };
        let db = db.clone();
        let mut tx = db.client().start_session().await?;
        tx.start_transaction()
            .write_concern(WriteConcern::majority())
            .and_run2(async move |tx| {
                transactions::transaction_result(
                    async {
                        // Claim and hint commit together. A later lifecycle action either
                        // replaces this generation or removes it, so stale repair loses.
                        if db
                            .collection::<Repair>(REPAIRS)
                            .delete_one(filter.clone())
                            .session(&mut *tx)
                            .await?
                            .deleted_count
                            == 0
                        {
                            return Ok(());
                        }
                        write_hint(&db, &repair.actor_id, hint.enabled, tx).await?;
                        for org in &hint.orgs {
                            jobs::enqueue(&db, org, tx).await?;
                        }
                        Ok(())
                    }
                    .await,
                )
            })
            .await
            .map_err(transactions::map_transaction_error)?;
    }
    Ok(())
}
