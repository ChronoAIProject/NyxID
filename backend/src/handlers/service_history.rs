use crate::{
    AppState,
    errors::AppResult,
    models::service_change_event::{HistoryActor, ServiceChangeEvent, ServiceChangeSummary},
    mw::auth::AuthUser,
    services::service_history::{definitions, read, relay},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::header,
};
use bson::doc;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ActorResponse {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub person_id: Option<String>,
    pub api_key_id: Option<String>,
    pub app_id: Option<String>,
}
impl From<&HistoryActor> for ActorResponse {
    fn from(a: &HistoryActor) -> Self {
        Self {
            kind: match a.kind {
                crate::models::service_change_event::HistoryActorKind::Person => "person",
                crate::models::service_change_event::HistoryActorKind::ApiKey => "api_key",
                crate::models::service_change_event::HistoryActorKind::ServiceAccount => {
                    "service_account"
                }
                crate::models::service_change_event::HistoryActorKind::App => "app",
                crate::models::service_change_event::HistoryActorKind::System => "system",
            }
            .into(),
            id: a.id.clone(),
            name: a.name.clone(),
            person_id: a.person_id.clone(),
            api_key_id: a.api_key_id.clone(),
            app_id: a.app_id.clone(),
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
pub struct SummaryResponse {
    pub actor: ActorResponse,
    pub at: String,
    pub action: String,
    pub action_label: String,
    pub change_group_id: String,
}
impl From<&ServiceChangeSummary> for SummaryResponse {
    fn from(s: &ServiceChangeSummary) -> Self {
        Self {
            actor: (&s.actor).into(),
            at: s.at.to_rfc3339(),
            action: s.action.clone(),
            action_label: definitions::action_label(&s.action).into(),
            change_group_id: s.change_group_id.clone(),
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorshipResponse {
    pub created_by: Option<SummaryResponse>,
    pub last_change: Option<SummaryResponse>,
}
#[derive(Serialize, ToSchema)]
pub struct FieldResponse {
    pub field: String,
    pub label: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}
#[derive(Serialize, ToSchema)]
pub struct EventResponse {
    pub id: String,
    pub schema_version: u32,
    pub service_slug: String,
    pub action: String,
    pub action_label: String,
    pub actor: ActorResponse,
    pub committed_at: String,
    pub changes: Vec<FieldResponse>,
    pub additional_changes: bool,
    pub audit_status: String,
}
#[derive(Serialize, ToSchema)]
pub struct GroupResponse {
    pub id: String,
    pub events: Vec<EventResponse>,
}
#[derive(Serialize, ToSchema)]
pub struct HistoryResponse {
    pub service_id: String,
    pub groups: Vec<GroupResponse>,
    pub next_cursor: Option<String>,
    pub tracked_since: Option<String>,
    pub legacy: bool,
    pub deleted: bool,
}
#[derive(Deserialize, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    pub cursor: Option<String>,
    pub actions: Option<String>,
    pub limit: Option<usize>,
}

pub async fn enrich_summaries(
    db: &mongodb::Database,
    auth: &AuthUser,
    keys: &mut [crate::handlers::keys::KeyResponse],
) -> AppResult<()> {
    let actor = auth.user_id.to_string();
    let reader = read::Reader {
        actor_id: &actor,
        allowed_service_ids: (!auth.allow_all_services)
            .then_some(auth.allowed_service_ids.as_slice()),
    };
    let ids: Vec<_> = keys.iter().map(|k| k.id.clone()).collect();
    let summaries = read::summaries(db, &reader, &ids).await?;
    for key in keys {
        key.authorship = summaries.get(&key.id).map(|s| AuthorshipResponse {
            created_by: s.created_by.as_ref().map(Into::into),
            last_change: s.last_change.as_ref().map(Into::into),
        });
    }
    Ok(())
}

fn event_response(
    mut event: ServiceChangeEvent,
    mirrors: &HashMap<String, crate::models::audit_log::AuditLog>,
    nodes: &HashSet<String>,
) -> EventResponse {
    let audit_status = if event.audited_at.is_none() {
        "pending"
    } else if mirrors
        .get(&event.id)
        .is_some_and(|mirror| relay::mirror_matches(&event, mirror))
    {
        "published"
    } else {
        "mismatch"
    };
    for change in &mut event.changes {
        if change.field == "node_id" {
            for value in [&mut change.before, &mut change.after] {
                if value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .is_some_and(|id| !nodes.contains(id))
                {
                    *value = None;
                }
            }
        }
    }
    EventResponse {
        id: event.id,
        schema_version: event.schema_version,
        service_slug: event.service_slug,
        action_label: definitions::action_label(&event.action).into(),
        action: event.action,
        actor: (&event.actor).into(),
        committed_at: event.committed_at.to_rfc3339(),
        additional_changes: event.additional_changes,
        changes: event
            .changes
            .into_iter()
            .filter_map(|c| {
                definitions::field(&c.field).map(|d| FieldResponse {
                    field: c.field,
                    label: d.label.into(),
                    before: c.before,
                    after: c.after,
                })
            })
            .collect(),
        audit_status: audit_status.into(),
    }
}

#[utoipa::path(get, path = "/api/v1/keys/{service_id}/history", params(("service_id" = String, Path), HistoryQuery), responses((status = 200, body = HistoryResponse), (status = 404, body = crate::errors::ErrorResponse)), security(("bearer_auth" = [])), tag = "AI Services")]
pub async fn get_history(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> AppResult<(
    [(header::HeaderName, &'static str); 1],
    Json<HistoryResponse>,
)> {
    let actor = auth.user_id.to_string();
    let reader = read::Reader {
        actor_id: &actor,
        allowed_service_ids: (!auth.allow_all_services)
            .then_some(auth.allowed_service_ids.as_slice()),
    };
    let actions: Vec<_> = query
        .actions
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let data = read::list(
        &state.db,
        &reader,
        &id,
        query.cursor.as_deref(),
        &actions,
        query.limit.unwrap_or(20),
    )
    .await?;
    let ids: Vec<_> = data
        .groups
        .iter()
        .flat_map(|(_, rows)| rows)
        .map(|e| &e.id)
        .collect();
    let mirrors: Vec<crate::models::audit_log::AuditLog> = state
        .db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! { "_id": { "$in": ids } })
        .await?
        .try_collect()
        .await?;
    let mirrors: HashMap<_, _> = mirrors.into_iter().map(|m| (m.id.clone(), m)).collect();
    let node_ids: Vec<_> = data
        .groups
        .iter()
        .flat_map(|(_, rows)| rows)
        .flat_map(|e| &e.changes)
        .filter(|c| c.field == "node_id")
        .flat_map(|c| [&c.before, &c.after])
        .filter_map(|v| v.as_ref().and_then(|v| v.as_str()))
        .collect();
    let nodes: Vec<crate::models::node::Node> = state
        .db
        .collection(crate::models::node::COLLECTION_NAME)
        .find(doc! { "_id": { "$in": node_ids }, "is_active": true })
        .await?
        .try_collect()
        .await?;
    let node_owners: Vec<_> = nodes.iter().map(|node| &node.user_id).collect();
    let active_owners: Vec<crate::models::user::User> = state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .find(doc! { "_id": { "$in": node_owners }, "is_active": true })
        .await?
        .try_collect()
        .await?;
    let active_owners: HashSet<_> = active_owners.into_iter().map(|owner| owner.id).collect();
    let mut allowed_nodes = HashSet::new();
    let mut owner_access = HashMap::new();
    for node in nodes {
        if !active_owners.contains(&node.user_id) {
            continue;
        }
        if !auth.allow_all_nodes && !auth.allowed_node_ids.contains(&node.id) {
            continue;
        }
        if !owner_access.contains_key(&node.user_id) {
            owner_access.insert(
                node.user_id.clone(),
                crate::services::org_service::resolve_owner_access(
                    &state.db,
                    &actor,
                    &node.user_id,
                )
                .await?,
            );
        }
        if owner_access
            .get(&node.user_id)
            .is_some_and(crate::services::node_service::node_access_can_read)
        {
            allowed_nodes.insert(node.id);
        }
    }
    let mut groups = Vec::new();
    for (id, rows) in data.groups {
        let mut events = Vec::new();
        for row in rows {
            events.push(event_response(row, &mirrors, &allowed_nodes));
        }
        groups.push(GroupResponse { id, events });
    }
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(HistoryResponse {
            service_id: id,
            groups,
            next_cursor: data.next_cursor,
            tracked_since: data.tracked_since.map(|d| d.to_rfc3339()),
            legacy: data.legacy,
            deleted: data.deleted,
        }),
    ))
}

#[derive(Serialize, ToSchema)]
pub struct ArchivedServiceResponse {
    pub service_id: String,
    pub service_slug: String,
    pub last_changed_at: String,
}
#[derive(Serialize, ToSchema)]
pub struct ArchiveResponse {
    pub services: Vec<ArchivedServiceResponse>,
    pub next_cursor: Option<String>,
}
#[derive(Deserialize, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
pub struct ArchiveQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}
#[utoipa::path(get, path = "/api/v1/keys/history/archived", params(ArchiveQuery), responses((status = 200, body = ArchiveResponse)), security(("bearer_auth" = [])), tag = "AI Services")]
pub async fn get_archived(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ArchiveQuery>,
) -> AppResult<(
    [(header::HeaderName, &'static str); 1],
    Json<ArchiveResponse>,
)> {
    let actor = auth.user_id.to_string();
    let reader = read::Reader {
        actor_id: &actor,
        allowed_service_ids: (!auth.allow_all_services)
            .then_some(auth.allowed_service_ids.as_slice()),
    };
    let data = read::archived(
        &state.db,
        &reader,
        query.cursor.as_deref(),
        query.limit.unwrap_or(20),
    )
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(ArchiveResponse {
            services: data
                .services
                .into_iter()
                .map(|s| ArchivedServiceResponse {
                    service_id: s.service_id,
                    service_slug: s.service_slug,
                    last_changed_at: s.last_changed_at.to_rfc3339(),
                })
                .collect(),
            next_cursor: data.next_cursor,
        }),
    ))
}

#[cfg(test)]
#[path = "service_history_tests.rs"]
mod tests;
