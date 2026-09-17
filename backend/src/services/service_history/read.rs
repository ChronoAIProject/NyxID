use crate::{
    errors::{AppError, AppResult},
    models::{
        org_membership::{OrgMembership, OrgRole},
        service_change_event::{COLLECTION_NAME, ServiceChangeEvent},
        user::{COLLECTION_NAME as USERS, User},
        user_service::UserService,
    },
    services::org_service,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::Database;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub struct Reader<'a> {
    pub actor_id: &'a str,
    pub allowed_service_ids: Option<&'a [String]>,
}
impl Reader<'_> {
    fn allows(&self, id: &str) -> bool {
        self.allowed_service_ids
            .is_none_or(|ids| ids.iter().any(|v| v == id))
    }
}

pub async fn can_read(
    db: &Database,
    reader: &Reader<'_>,
    owner: &str,
    id: &str,
) -> AppResult<bool> {
    if !reader.allows(id) {
        return Ok(false);
    }
    let owner_active = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": owner, "is_active": true })
        .await?
        .is_some();
    if !owner_active {
        return Ok(false);
    }
    let access = org_service::resolve_owner_access(db, reader.actor_id, owner).await?;
    Ok(access.can_write() && access.allows_resource(id))
}

pub async fn summaries(
    db: &Database,
    reader: &Reader<'_>,
    ids: &[String],
    memberships: &[OrgMembership],
) -> AppResult<HashMap<String, UserService>> {
    let services: Vec<UserService> = db
        .collection::<UserService>("user_services")
        .find(doc! { "_id": { "$in": ids } })
        .await?
        .try_collect()
        .await?;
    let owners: Vec<_> = services.iter().map(|s| s.user_id.as_str()).collect();
    let active: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": owners }, "is_active": true })
        .await?
        .try_collect()
        .await?;
    let mut scopes = HashMap::new();
    for owner in active {
        if owner.id == reader.actor_id {
            scopes.insert(owner.id, None);
        } else if owner.user_type.is_org()
            && let Some(membership) = memberships.iter().find(|m| {
                m.org_user_id == owner.id
                    && m.member_user_id == reader.actor_id
                    && m.revoked_at.is_none()
                    && m.role == OrgRole::Admin
            })
        {
            let scope = crate::services::org_role_scope_service::effective_scope_for_membership(
                db, membership,
            )
            .await?;
            scopes.insert(owner.id, scope);
        }
    }
    Ok(services
        .into_iter()
        .filter(|s| {
            reader.allows(&s.id)
                && scopes.get(&s.user_id).is_some_and(|scope| {
                    crate::services::org_role_scope_service::scope_allows(scope, &s.id)
                })
        })
        .map(|s| (s.id.clone(), s))
        .collect())
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    sequence: i64,
    group: String,
}

pub struct HistoryData {
    pub groups: Vec<(String, Vec<ServiceChangeEvent>)>,
    pub next_cursor: Option<String>,
    pub tracked_since: Option<chrono::DateTime<chrono::Utc>>,
    pub legacy: bool,
    pub deleted: bool,
}

pub async fn list(
    db: &Database,
    reader: &Reader<'_>,
    id: &str,
    cursor: Option<&str>,
    actions: &[String],
    limit: usize,
) -> AppResult<HistoryData> {
    if uuid::Uuid::parse_str(id).is_err()
        || id.len() != 36
        || !(1..=50).contains(&limit)
        || actions.len() > 20
    {
        return Err(AppError::ValidationError(
            "History requires a service UUID, limit 1–50, and at most 20 actions".into(),
        ));
    }
    // Unknown historical codes are accepted as filters, never as mutation input.
    if actions.iter().any(|v| {
        v.len() > 80
            || !v
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    }) {
        return Err(AppError::ValidationError(
            "Invalid history action filter".into(),
        ));
    }
    let events = db.collection::<ServiceChangeEvent>(COLLECTION_NAME);
    let service = db
        .collection::<UserService>("user_services")
        .find_one(doc! { "_id": id })
        .await?;
    let first = events
        .find_one(doc! { "service_id": id })
        .sort(doc! { "service_sequence": 1 })
        .await?;
    let owner = service
        .as_ref()
        .map(|s| s.user_id.as_str())
        .or_else(|| first.as_ref().map(|e| e.owner_id.as_str()))
        .ok_or_else(|| AppError::NotFound("Service history not found".into()))?;
    if !can_read(db, reader, owner, id).await? {
        return Err(AppError::NotFound("Service history not found".into()));
    }
    let mut pipeline = vec![
        doc! { "$match": { "service_id": id, "owner_id": owner } },
        doc! { "$group": { "_id": "$change_group_id", "sequence": { "$min": "$service_sequence" }, "actions": { "$addToSet": "$action" }, "events": { "$sum": 1 } } },
    ];
    if !actions.is_empty() {
        pipeline.push(doc! { "$match": { "actions": { "$in": actions } } });
    }
    if let Some(cursor) = cursor {
        if cursor.len() > 512 {
            return Err(AppError::ValidationError("Invalid history cursor".into()));
        }
        let cursor: Cursor = URL_SAFE_NO_PAD
            .decode(cursor)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .ok_or_else(|| AppError::ValidationError("Invalid history cursor".into()))?;
        if uuid::Uuid::parse_str(&cursor.group).is_err() {
            return Err(AppError::ValidationError("Invalid history cursor".into()));
        }
        pipeline.push(doc! { "$match": { "$or": [ { "sequence": { "$lt": cursor.sequence } }, { "sequence": cursor.sequence, "_id": { "$lt": cursor.group } } ] } });
    }
    pipeline.extend([
        doc! { "$sort": { "sequence": -1, "_id": -1 } },
        doc! { "$limit": (limit + 1) as i64 },
    ]);
    let mut groups: Vec<Document> = events.aggregate(pipeline).await?.try_collect().await?;
    // Target 1,000 events while retaining whole operations. A single larger
    // operation is returned alone, so even older oversized groups stay readable.
    let mut event_count = 0_i64;
    let mut take = 0;
    for group in groups.iter().take(limit) {
        let count = group
            .get_i32("events")
            .map(i64::from)
            .or_else(|_| group.get_i64("events"))
            .unwrap_or(1);
        if take > 0 && event_count + count > 1_000 {
            break;
        }
        take += 1;
        event_count += count;
    }
    let has_more = groups.len() > take;
    groups.truncate(take);
    let next_cursor = if has_more {
        groups
            .last()
            .map(|g| Cursor {
                sequence: g.get_i64("sequence").expect("group sequence"),
                group: g.get_str("_id").expect("group id").into(),
            })
            .map(|v| {
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&v).expect("cursor is serializable"))
            })
    } else {
        None
    };
    let group_ids: Vec<_> = groups
        .iter()
        .filter_map(|g| g.get_str("_id").ok())
        .collect();
    let rows: Vec<ServiceChangeEvent> = events
        .find(
            doc! { "service_id": id, "owner_id": owner, "change_group_id": { "$in": &group_ids } },
        )
        .sort(doc! { "service_sequence": 1 })
        .batch_size(128)
        .await?
        .try_collect()
        .await?;
    let mut grouped: HashMap<String, Vec<ServiceChangeEvent>> = HashMap::new();
    for row in rows {
        grouped
            .entry(row.change_group_id.clone())
            .or_default()
            .push(row);
    }
    let deleted = service.as_ref().is_none_or(|s| s.deleted_at.is_some());
    Ok(HistoryData {
        groups: group_ids
            .into_iter()
            .map(|g| (g.into(), grouped.remove(g).unwrap_or_default()))
            .collect(),
        next_cursor,
        tracked_since: first.as_ref().map(|e| e.committed_at),
        legacy: service.as_ref().map_or_else(
            || first.as_ref().is_none_or(|e| e.action != "service.created"),
            |s| s.created_by.is_none(),
        ),
        deleted,
    })
}

pub struct ArchivedService {
    pub service_id: String,
    pub service_slug: String,
    pub last_changed_at: chrono::DateTime<chrono::Utc>,
}
pub struct ArchiveData {
    pub services: Vec<ArchivedService>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct ArchiveCursor {
    at: i64,
    service: String,
}

/// Discover retained UUIDs using today's owner, role and effective resource scope.
pub async fn archived(
    db: &Database,
    reader: &Reader<'_>,
    cursor: Option<&str>,
    limit: usize,
) -> AppResult<ArchiveData> {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership};
    if !(1..=50).contains(&limit) {
        return Err(AppError::ValidationError(
            "Archive limit must be 1–50".into(),
        ));
    }
    let memberships: Vec<OrgMembership> = db
        .collection(MEMBERSHIPS)
        .find(doc! { "member_user_id": reader.actor_id, "role": "admin", "revoked_at": null })
        .await?
        .try_collect()
        .await?;
    let owners: Vec<_> = std::iter::once(reader.actor_id)
        .chain(memberships.iter().map(|m| m.org_user_id.as_str()))
        .collect();
    let active: Vec<User> = db
        .collection(USERS)
        .find(doc! { "_id": { "$in": owners }, "is_active": true })
        .await?
        .try_collect()
        .await?;
    let mut permitted = Vec::new();
    for owner in active {
        let access = org_service::resolve_owner_access(db, reader.actor_id, &owner.id).await?;
        if !access.can_write() {
            continue;
        }
        let mut filter = doc! { "owner_id": &owner.id };
        if let org_service::OwnerAccess::AsOrgAdmin {
            allowed_service_ids: Some(ids),
            ..
        } = access
        {
            let ids: Vec<_> = ids.into_iter().filter(|id| reader.allows(id)).collect();
            filter.insert("service_id", doc! { "$in": ids });
        } else if let Some(ids) = reader.allowed_service_ids {
            filter.insert("service_id", doc! { "$in": ids });
        }
        permitted.push(filter);
    }
    if permitted.is_empty() {
        return Ok(ArchiveData {
            services: vec![],
            next_cursor: None,
        });
    }
    let mut pipeline = vec![
        doc! { "$match": { "$or": permitted } },
        doc! { "$group": { "_id": "$service_id", "at": { "$max": "$committed_at" }, "snapshot": { "$top": { "sortBy": { "service_sequence": -1 }, "output": "$service_slug" } } } },
        doc! { "$lookup": { "from": "user_services", "localField": "_id", "foreignField": "_id", "as": "live", "pipeline": [{ "$project": { "deleted_at": 1 } }] } },
        doc! { "$match": { "$or": [ { "live": { "$size": 0 } }, { "live.deleted_at": { "$type": "date" } } ] } },
    ];
    if let Some(cursor) = cursor {
        if cursor.len() > 512 {
            return Err(AppError::ValidationError("Invalid archive cursor".into()));
        }
        let cursor: ArchiveCursor = URL_SAFE_NO_PAD
            .decode(cursor)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| AppError::ValidationError("Invalid archive cursor".into()))?;
        if uuid::Uuid::parse_str(&cursor.service).is_err() {
            return Err(AppError::ValidationError("Invalid archive cursor".into()));
        }
        let at = bson::DateTime::from_millis(cursor.at);
        pipeline.push(doc! { "$match": { "$or": [ { "at": { "$lt": at } }, { "at": at, "_id": { "$lt": cursor.service } } ] } });
    }
    pipeline.extend([
        doc! { "$sort": { "at": -1, "_id": -1 } },
        doc! { "$limit": (limit+1) as i64 },
        doc! { "$project": { "snapshot": 1, "at": 1 } },
    ]);
    let mut rows: Vec<Document> = db
        .collection::<ServiceChangeEvent>(COLLECTION_NAME)
        .aggregate(pipeline)
        .allow_disk_use(true)
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if more {
        rows.last().map(|row| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&ArchiveCursor {
                    at: row
                        .get_datetime("at")
                        .expect("archive date")
                        .timestamp_millis(),
                    service: row.get_str("_id").expect("archive UUID").into(),
                })
                .expect("archive cursor"),
            )
        })
    } else {
        None
    };
    Ok(ArchiveData {
        services: rows
            .into_iter()
            .map(|row| ArchivedService {
                service_id: row.get_str("_id").expect("archive UUID").into(),
                service_slug: row.get_str("snapshot").unwrap_or("service").into(),
                last_changed_at: row.get_datetime("at").expect("archive date").to_chrono(),
            })
            .collect(),
        next_cursor,
    })
}
