//! Per-specialist policy, shared by the human API and NyxBot's confirmed tool.
use std::collections::BTreeMap;

use bson::doc;
use futures::TryStreamExt;
use mongodb::Database;
use serde::{Deserialize, Serialize};

use super::{api_key_mutation_service as transactions, assistant_team_service as team};
use crate::{
    errors::{AppError, AppResult},
    models::assistant_agent::{
        AssistantAgent, GuestAccess, GuestDefaultAccess, GuestEscalation, GuestPolicy,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceLevel {
    Read,
    Use,
    All,
    Default,
}
impl ServiceLevel {
    pub(crate) fn explicit(self) -> Option<GuestAccess> {
        match self {
            Self::Read => Some(GuestAccess::Read),
            Self::Use => Some(GuestAccess::Use),
            Self::All => Some(GuestAccess::All),
            Self::Default => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub enabled: Option<bool>,
    pub default_access: Option<GuestDefaultAccess>,
    pub escalation: Option<GuestEscalation>,
    pub share_skills: Option<bool>,
    #[serde(default)]
    pub services: BTreeMap<String, ServiceLevel>,
}

#[derive(Serialize)]
pub struct ServiceAccess {
    pub access: GuestAccess,
    pub source: &'static str,
}
#[derive(Serialize)]
pub struct View {
    pub agent_id: String,
    pub guest_policy: GuestPolicy,
    pub effective_guest_policy: GuestPolicy,
    pub services: BTreeMap<String, ServiceAccess>,
}

pub(crate) async fn require_guest_specialist(
    db: &Database,
    agent: &AssistantAgent,
) -> AppResult<()> {
    let owner = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": &agent.user_id})
        .await?;
    if owner.is_some_and(|owner| owner.user_type.is_org()) {
        return Err(AppError::Forbidden(
            "Organization specialists do not answer guests".into(),
        ));
    }
    Ok(())
}

pub async fn view(db: &Database, agent: &AssistantAgent) -> AppResult<View> {
    require_guest_specialist(db, agent).await?;
    if agent.is_nyxbot() {
        return Err(AppError::ValidationError(
            "Guest policy belongs to specialists".into(),
        ));
    }
    let mut catalogs = BTreeMap::new();
    if !agent.grants.service_ids.is_empty() {
        let mut cursor = db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .find(doc! {"_id": {"$in": &agent.grants.service_ids}})
            .projection(doc! {"catalog_service_id": 1})
            .await?;
        while let Some(row) = cursor.try_next().await? {
            if let (Ok(id), Ok(catalog)) = (row.get_str("_id"), row.get_str("catalog_service_id")) {
                catalogs.insert(id.to_owned(), catalog.to_owned());
            }
        }
    }
    Ok(View {
        agent_id: agent.id.clone(),
        guest_policy: agent.guest_policy.clone(),
        effective_guest_policy: agent.effective_guest_policy(),
        services: agent
            .grants
            .service_ids
            .iter()
            .chain(&agent.grants.platform_service_ids)
            .map(|id| {
                let catalog = catalogs.get(id).map(String::as_str);
                (
                    id.clone(),
                    ServiceAccess {
                        access: agent.guest_level_for(id, catalog),
                        source: if agent.guest_access.contains_key(id)
                            || catalog.is_some_and(|id| agent.guest_access.contains_key(id))
                        {
                            "explicit"
                        } else {
                            "default"
                        },
                    },
                )
            })
            .collect(),
    })
}

fn audit_snapshot(agent: &AssistantAgent) -> serde_json::Value {
    serde_json::json!({"guest_policy": agent.guest_policy, "guest_access": agent.guest_access})
}

/// Policy selectors can only name an existing grant. Slug selection follows
/// guest MCP discovery; exact IDs remain selectable when a slug is shared.
pub(crate) async fn resolve_each_target(
    db: &Database,
    node_manager: &super::node_ws_manager::NodeWsManager,
    agent: &AssistantAgent,
    requested: &[String],
) -> AppResult<(BTreeMap<String, String>, Vec<team::Refused>)> {
    if requested.len() > team::MAX_GRANT_TARGETS {
        return Err(AppError::ValidationError(format!(
            "At most {} service levels may be changed",
            team::MAX_GRANT_TARGETS
        )));
    }
    let granted = |id: &str| {
        agent.grants.service_ids.iter().any(|value| value == id)
            || agent
                .grants
                .platform_service_ids
                .iter()
                .any(|value| value == id)
    };
    let catalog = if requested.iter().any(|target| !granted(target.trim())) {
        Some(
            Box::pin(super::mcp_service::load_operation_catalog(
                db,
                node_manager,
                &agent.user_id,
                super::mcp_service::NodeScope::Unrestricted,
                super::mcp_service::ServiceScope::Granted {
                    user_service_ids: &agent.grants.service_ids,
                    platform_service_ids: &agent.grants.platform_service_ids,
                },
            ))
            .await?,
        )
    } else {
        None
    };
    let mut targets = BTreeMap::new();
    let mut refused = Vec::new();
    for target in requested {
        let target = target.trim();
        let id = if granted(target) {
            Some(target)
        } else {
            catalog.as_ref().and_then(|catalog| {
                catalog
                    .services
                    .iter()
                    .find(|service| service.service_slug == target)
                    .map(|service| service.service_id.as_str())
            })
        };
        if let Some(id) = id {
            targets.insert(target.to_owned(), id.to_owned());
        } else {
            refused.push(team::Refused {
                service: super::assistant_nyxagent::identifier(target),
                reason: "Not granted or unavailable to this specialist; use a granted service ID or grant it first with nyxid__grant_subagent".into(),
            });
        }
    }
    Ok((targets, refused))
}

pub async fn update(
    db: &Database,
    node_manager: &super::node_ws_manager::NodeWsManager,
    actor: &super::audit_service::AuditActor,
    id: &str,
    patch: Patch,
) -> AppResult<AssistantAgent> {
    let current = team::maintained_agent(db, &actor.user_id, id).await?;
    require_guest_specialist(db, &current).await?;
    if current.is_nyxbot() {
        return Err(AppError::ValidationError(
            "Guest policy belongs to specialists".into(),
        ));
    }
    let requested: Vec<String> = patch.services.keys().cloned().collect();
    let (targets, refused) =
        Box::pin(resolve_each_target(db, node_manager, &current, &requested)).await?;
    if let Some(refused) = refused.into_iter().next() {
        return Err(AppError::ValidationError(refused.reason));
    }
    let db_owned = db.clone();
    let actor_owned = actor.user_id.clone();
    let id_owned = id.to_owned();
    let mut session = db.client().start_session().await?;
    let (old, agent) = session.start_transaction().and_run2(async move |session| {
        let result = async {

            let collection = db_owned.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME);
            let filter = doc! {"_id": &id_owned, "user_id": &actor_owned, "kind": "specialist", "destroyed_at": bson::Bson::Null};
            let mut agent = collection.find_one(filter.clone()).session(&mut *session).await?
                .ok_or_else(|| AppError::NotFound("Specialist not found".into()))?;
            // Guest settings are personal-only. Re-read and fence the actual owner
            // in this transaction; a preflight maintainer read grants no write authority.
            super::org_membership_authority::fence_live_member(&db_owned, &actor_owned, &mut *session).await?;
            super::erasure_admission::fence_in_session(&db_owned, &actor_owned, &mut *session).await?;
            let old = audit_snapshot(&agent);
            if let Some(value) = patch.enabled { agent.guest_policy.enabled = value; }
            if let Some(value) = patch.default_access { agent.guest_policy.default_access = value; }
            if let Some(value) = patch.escalation { agent.guest_policy.escalation = value; }
            if let Some(value) = patch.share_skills { agent.guest_policy.share_skills = value; }
            for (name, level) in &patch.services {
                let id = targets.get(name.trim()).ok_or_else(|| AppError::ValidationError("Service not found".into()))?;
                if !agent.grants.service_ids.contains(id) && !agent.grants.platform_service_ids.contains(id) {
                    return Err(AppError::ValidationError("Guest access requires a granted service".into()));
                }
                if let Some(level) = level.explicit() { agent.guest_access.insert(id.clone(), level); }
                else { agent.guest_access.remove(id); }
            }
            let encode = |value| bson::to_bson(value).map_err(|_| AppError::Internal("Guest policy encoding failed".into()));
            collection.update_one(filter, doc! {"$set": {
                "guest_policy": encode(&agent.guest_policy)?,
                "guest_access": bson::to_bson(&agent.guest_access).map_err(|_| AppError::Internal("Guest level encoding failed".into()))?,
                "updated_at": bson::DateTime::now(),
            }}).session(&mut *session).await?;
            Ok((old, agent))
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        actor,
        "assistant_guest_policy_changed",
        Some(serde_json::json!({"agent_id": id, "old": old, "new": audit_snapshot(&agent), "actor": actor.user_id})),
    )
    .await;
    Ok(agent)
}
