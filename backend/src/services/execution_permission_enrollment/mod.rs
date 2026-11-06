//! Rollout hints only. Execution permissions and their existing claims remain authority.
//! Fresh candidacy is DB-free; admission and marked Phase B barriers resolve live flags.
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};

use super::{api_key_mutation_service as transactions, feature_flag_service as flags};
use crate::{
    errors::{AppError, AppResult},
    models::{
        feature_flag_override::{
            COLLECTION_NAME as OVERRIDES, FeatureFlagOverride, FlagTargetKind,
        },
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole},
        user::{COLLECTION_NAME as USERS, User},
    },
};

pub(crate) mod jobs;
pub(crate) mod lifecycle;
pub(crate) mod snapshot;

pub const READINESS: &str = "permissions:execution-enrollment-v1";
pub const FLAG: &str = flags::EXECUTION_PERMISSIONS_FLAG_KEY;
const MAX_MEMBERSHIPS: usize = 256;
const MAX_SCOPE_ROWS: usize = 2048;
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

fn limit_error() -> AppError {
    AppError::Internal("Execution enrollment lookup exceeded its bounded scope; retry after narrowing organization memberships or overrides".into())
}

/// Reuse the generic precedence model, including legacy role/org-user rows and
/// deny-wins ties. The compiled flag cannot be shadowed by a runtime definition.
pub(crate) fn resolve(
    rows: &[FeatureFlagOverride],
    memberships: &[(String, OrgRole)],
    person: &str,
    include_global: bool,
) -> bool {
    let (org, platform): (Vec<_>, Vec<_>) = rows
        .iter()
        .filter(|r| r.flag_key == FLAG && r.definition_id.is_none())
        .filter(|r| include_global || r.target_kind != FlagTargetKind::Global)
        .cloned()
        .partition(|r| r.org_user_id.is_some());
    !flags::resolve_personal_definitions(
        [(FLAG, false, None)],
        &platform,
        &org,
        memberships,
        person,
    )
    .is_empty()
}

async fn memberships(
    db: &Database,
    actor: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<Option<Vec<(String, OrgRole)>>> {
    let collection = db.collection::<OrgMembership>(MEMBERSHIPS);
    let query = collection
        .find(doc! {"member_user_id":actor,"revoked_at":null})
        .limit(MAX_MEMBERSHIPS as i64 + 1)
        .max_time(READ_TIMEOUT);
    let rows: Vec<OrgMembership> = if let Some(session) = session {
        query
            .session(&mut *session)
            .await?
            .stream(session)
            .try_collect()
            .await?
    } else {
        query.await?.try_collect().await?
    };
    if rows.len() > MAX_MEMBERSHIPS {
        return Ok(None);
    }
    Ok(Some(
        rows.into_iter().map(|m| (m.org_user_id, m.role)).collect(),
    ))
}

async fn rows(
    db: &Database,
    actor: &str,
    memberships: &[(String, OrgRole)],
    session: Option<&mut ClientSession>,
) -> AppResult<Option<Vec<FeatureFlagOverride>>> {
    let orgs: Vec<_> = memberships.iter().map(|(id, _)| id).collect();
    let collection = db.collection::<FeatureFlagOverride>(OVERRIDES);
    let query = collection
        .find(doc! {
            "flag_key":FLAG,"definition_id":null,"$or":[
                {"org_user_id":null,"target_kind":"global"},
                {"org_user_id":null,"target_kind":"user","target_key":actor},
                {"org_user_id":{"$in":orgs},"$or":[
                    {"target_kind":"org"},
                    {"target_kind":"role","target_key":{"$in":["admin","member","viewer"]}},
                    {"target_kind":"user","target_key":actor}
                ]}
            ]
        })
        .limit(MAX_SCOPE_ROWS as i64 + 1)
        .max_time(READ_TIMEOUT);
    let rows: Vec<FeatureFlagOverride> = if let Some(session) = session {
        query
            .session(&mut *session)
            .await?
            .stream(session)
            .try_collect()
            .await?
    } else {
        query.await?.try_collect().await?
    };
    if rows.len() > MAX_SCOPE_ROWS {
        return Ok(None);
    }
    Ok(Some(rows))
}

// A personal override wins even if the lower-precedence org set overflows.
// Keep the common path at three reads; this extra lookup is only for overflow.
async fn scoped_enabled(
    db: &Database,
    actor: &str,
    include_global: bool,
    mut session: Option<&mut ClientSession>,
) -> AppResult<Option<bool>> {
    if let Some(memberships) = memberships(db, actor, session.as_deref_mut()).await?
        && let Some(rows) = rows(db, actor, &memberships, session.as_deref_mut()).await?
    {
        return Ok(Some(resolve(&rows, &memberships, actor, include_global)));
    }
    let collection = db.collection::<FeatureFlagOverride>(OVERRIDES);
    let query = collection.find_one(doc! {
        "flag_key":FLAG,"definition_id":null,"org_user_id":null,"target_kind":"user","target_key":actor
    }).max_time(READ_TIMEOUT);
    let personal = if let Some(session) = session {
        query.session(session).await?
    } else {
        query.await?
    };
    Ok(personal.map(|row| row.enabled))
}

/// For pre-participation admission only: false means use the legacy path.
/// Already-marked work must interpret false as drain/refusal, never downgrade.
/// No readiness/permission authority is conferred by this boolean.
pub(crate) async fn live_enabled(
    db: &Database,
    actor: &str,
    mut session: Option<&mut ClientSession>,
) -> AppResult<bool> {
    let collection = db.collection::<Document>(USERS);
    let query = collection.find_one(doc! {
        "_id":actor,"user_type":"person","is_active":true,"platform_personal_deleting":{"$ne":true}
    }).projection(doc! {"_id":1}).max_time(READ_TIMEOUT);
    let active = if let Some(tx) = session.as_deref_mut() {
        query.session(tx).await?
    } else {
        query.await?
    };
    if active.is_none() {
        return Ok(false);
    }
    scoped_enabled(db, actor, true, session)
        .await?
        .ok_or_else(limit_error)
}

/// Person markers materialize personal/org enrollment only. Global candidacy is
/// exclusively the process snapshot; changing it never queues a population scan.
pub(crate) async fn recompute_in_session(
    db: &Database,
    actor: &str,
    session: &mut ClientSession,
) -> AppResult<Option<u32>> {
    let person = db
        .collection::<Document>(USERS)
        .find_one(doc! {"_id":actor})
        .projection(doc! {"user_type":1,"is_active":1,"platform_personal_deleting":1})
        .session(&mut *session)
        .await?;
    let Some(person) = person else {
        return Ok(None);
    };
    let active = person.get_str("user_type").ok() == Some("person")
        && person.get_bool("is_active").unwrap_or(false)
        && !person
            .get_bool("platform_personal_deleting")
            .unwrap_or(false);
    let enrolled = if active {
        // Overflow cannot block a membership departure/disable. A missing hint
        // only selects legacy; live admission still refuses an incomplete read.
        scoped_enabled(db, actor, false, Some(&mut *session))
            .await?
            .unwrap_or(false)
    } else {
        false
    };
    let update = if enrolled {
        doc! {"$set":{"execution_permission_enrollment":1_i32}}
    } else {
        doc! {"$unset":{"execution_permission_enrollment":""}}
    };
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":actor}, update)
        .session(session)
        .await?;
    Ok(enrolled.then_some(1))
}

#[cfg(test)]
mod tests;
