//! Shared recipient policy validation and person-only membership queries.
use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};

use crate::errors::{AppError, AppResult};
use crate::models::billing_target::BillingTargetKind;
use crate::models::group::COLLECTION_NAME as GROUPS;
use crate::models::org_membership::COLLECTION_NAME as MEMBERSHIPS;
use crate::models::user::COLLECTION_NAME as USERS;

/// Each explicit list is bounded independently of the number of resolved people.
pub(super) const MAX_SELECTED_TARGETS: usize = super::grants::MAX_SELECTED_USERS;

pub(super) fn validate_shape(
    kind: BillingTargetKind,
    users: &[String],
    orgs: &[String],
    groups: &[String],
) -> AppResult<()> {
    for (expected, name, ids) in [
        (BillingTargetKind::SelectedUsers, "target_user_ids", users),
        (BillingTargetKind::OrgMembers, "target_org_ids", orgs),
        (BillingTargetKind::Groups, "target_group_ids", groups),
    ] {
        if kind != expected {
            if !ids.is_empty() {
                return Err(AppError::ValidationError(format!(
                    "{name} must be empty for this target_kind"
                )));
            }
            continue;
        }
        if ids.is_empty() || ids.len() > MAX_SELECTED_TARGETS {
            return Err(AppError::ValidationError(format!(
                "{name} must contain 1-{MAX_SELECTED_TARGETS} ids"
            )));
        }
        let unique: BTreeSet<_> = ids.iter().collect();
        if unique.len() != ids.len() || ids.iter().any(|id| id.is_empty() || id != id.trim()) {
            return Err(AppError::ValidationError(format!(
                "{name} must be unique, non-empty, and without surrounding whitespace"
            )));
        }
    }
    Ok(())
}

pub(super) async fn validate(
    db: &mongodb::Database,
    kind: BillingTargetKind,
    users: &[String],
    orgs: &[String],
    groups: &[String],
) -> AppResult<()> {
    validate_shape(kind, users, orgs, groups)?;
    validate_existing(db, kind, users, orgs, groups).await
}

/// Validate referenced rows after the caller has validated the list shape.
pub(super) async fn validate_existing(
    db: &mongodb::Database,
    kind: BillingTargetKind,
    users: &[String],
    orgs: &[String],
    groups: &[String],
) -> AppResult<()> {
    let (collection, ids, mut filter, error) = match kind {
        BillingTargetKind::AllUsers => return Ok(()),
        BillingTargetKind::SelectedUsers => (
            USERS,
            users,
            doc! { "is_active": true },
            "one or more target users do not exist or are inactive",
        ),
        BillingTargetKind::OrgMembers => (
            USERS,
            orgs,
            doc! { "is_active": true, "user_type": "org" },
            "one or more target organizations do not exist, are inactive, or are not organizations",
        ),
        BillingTargetKind::Groups => (
            GROUPS,
            groups,
            doc! {},
            "one or more target groups do not exist",
        ),
    };
    filter.insert("_id", doc! { "$in": ids });
    if db
        .collection::<Document>(collection)
        .count_documents(filter)
        .await?
        != ids.len() as u64
    {
        return Err(AppError::ValidationError(error.to_string()));
    }
    Ok(())
}

/// Kind changes require a new matching list and discard omitted old lists.
/// Explicit incompatible lists still fail validation instead of being ignored.
pub(super) fn updated_lists(
    current_kind: BillingTargetKind,
    kind: BillingTargetKind,
    current: [&[String]; 3],
    requested: [Option<Vec<String>>; 3],
) -> AppResult<[Vec<String>; 3]> {
    let changed = current_kind != kind;
    let matching = match kind {
        BillingTargetKind::AllUsers => None,
        BillingTargetKind::SelectedUsers => Some(0),
        BillingTargetKind::OrgMembers => Some(1),
        BillingTargetKind::Groups => Some(2),
    };
    if changed && matching.is_some_and(|index| requested[index].is_none()) {
        return Err(AppError::ValidationError(
            "changing target_kind requires its matching target id list".to_string(),
        ));
    }
    let mut index = 0;
    let result = requested.map(|ids| {
        let value = ids.unwrap_or_else(|| {
            if changed {
                Vec::new()
            } else {
                current[index].to_vec()
            }
        });
        index += 1;
        value
    });
    validate_shape(kind, &result[0], &result[1], &result[2])?;
    Ok(result)
}

/// Walk distinct member ids for organizations and indexed users for groups.
/// Both paths return unique user ids in ascending cursor order.
pub(super) async fn member_recipients(
    db: &mongodb::Database,
    kind: BillingTargetKind,
    orgs: &[String],
    groups: &[String],
    cutoff: Option<DateTime<Utc>>,
    cursor: Option<&str>,
    limit: usize,
) -> AppResult<Vec<String>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    // Missing user_type is the legacy spelling of person.
    let mut filter = doc! { "is_active": true, "user_type": { "$ne": "org" } };
    if let Some(cutoff) = cutoff {
        filter.insert(
            "created_at",
            doc! { "$lte": bson::DateTime::from_chrono(cutoff) },
        );
    }
    let (collection, mut pipeline) = if kind == BillingTargetKind::OrgMembers {
        let mut membership =
            doc! { "org_user_id": { "$in": orgs }, "revoked_at": bson::Bson::Null };
        if let Some(cutoff) = cutoff {
            membership.insert(
                "created_at",
                doc! { "$lte": bson::DateTime::from_chrono(cutoff) },
            );
        }
        if let Some(cursor) = cursor {
            membership.insert("member_user_id", doc! { "$gt": cursor });
        }
        (
            MEMBERSHIPS,
            vec![
                doc! { "$match": membership },
                doc! { "$group": { "_id": "$member_user_id" } },
                doc! { "$sort": { "_id": 1 } },
                doc! { "$lookup": {
                    "from": USERS, "localField": "_id", "foreignField": "_id",
                    "pipeline": [{ "$match": filter }, { "$project": { "_id": 1 } }],
                    "as": "eligible_users",
                } },
                doc! { "$match": { "eligible_users.0": { "$exists": true } } },
            ],
        )
    } else {
        if let Some(cursor) = cursor {
            filter.insert("_id", doc! { "$gt": cursor });
        }
        if kind == BillingTargetKind::Groups {
            filter.insert("group_ids", doc! { "$in": groups });
        }
        (
            USERS,
            vec![doc! { "$match": filter }, doc! { "$sort": { "_id": 1 } }],
        )
    };
    pipeline.extend([
        doc! { "$limit": limit as i64 },
        doc! { "$project": { "_id": 1 } },
    ]);
    db.collection::<Document>(collection)
        .aggregate(pipeline)
        .await?
        .map_ok(|row| {
            row.get_str("_id")
                .map(str::to_owned)
                .map_err(|error| AppError::Internal(format!("invalid recipient id: {error}")))
        })
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .collect()
}

/// Load live owner targets in one aggregation, shared by display, reservation
/// and settlement. Org wallets never inherit person benefits.
pub(super) async fn allowance_clauses(
    db: &mongodb::Database,
    owner_id: &str,
) -> AppResult<Vec<Document>> {
    let mut clauses = vec![
        doc! { "target_kind": "all_users" },
        doc! { "target_kind": "selected_users", "target_user_ids": owner_id },
    ];
    let Some(owner) = db
        .collection::<Document>(USERS)
        .aggregate([
            doc! { "$match": { "_id": owner_id, "is_active": true } },
            doc! { "$lookup": {
                "from": MEMBERSHIPS, "localField": "_id", "foreignField": "member_user_id",
                "pipeline": [
                    { "$match": { "revoked_at": bson::Bson::Null } },
                    { "$lookup": {
                        "from": USERS, "localField": "org_user_id", "foreignField": "_id",
                        "pipeline": [
                            { "$match": { "is_active": true, "user_type": "org" } },
                            { "$project": { "_id": 1 } },
                        ],
                        "as": "active_org",
                    } },
                    { "$match": { "active_org.0": { "$exists": true } } },
                    { "$group": { "_id": "$org_user_id" } },
                ],
                "as": "memberships",
            } },
            doc! { "$project": {
                "user_type": 1,
                "group_ids": { "$ifNull": ["$group_ids", []] },
                "org_ids": "$memberships._id",
            } },
        ])
        .await?
        .try_next()
        .await?
    else {
        return Ok(clauses);
    };
    if owner.get_str("user_type") == Ok("org") {
        return Ok(clauses);
    }
    for (field, kind, target_field) in [
        ("org_ids", "org_members", "target_org_ids"),
        ("group_ids", "groups", "target_group_ids"),
    ] {
        if let Ok(ids) = owner.get_array(field)
            && !ids.is_empty()
        {
            clauses.push(doc! { "target_kind": kind, target_field: { "$in": ids } });
        }
    }
    Ok(clauses)
}

#[cfg(test)]
mod tests;
