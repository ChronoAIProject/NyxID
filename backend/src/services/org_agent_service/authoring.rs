//! Governance of the two independent organization specialist access lists.
use super::{MemberFence, live_specialist_filter, member_rights_in_session};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS},
        org_membership::{COLLECTION_NAME as MEMBERS, OrgRole},
        user::COLLECTION_NAME as USERS,
    },
    services::{
        audit_service::{AuditActor, required},
        feature_flag_service as flags, security_effect_service, security_readiness_service,
        transaction_service,
    },
};
use bson::{Document, doc};
use mongodb::{ClientSession, Database, options::ReturnDocument};
use serde_json::json;
use std::collections::BTreeSet;

pub(crate) async fn available(db: &Database, actor: &str, org: &str) -> AppResult<bool> {
    let enabled = flags::resolve_enabled_features(db, org, actor, OrgRole::Admin).await?;
    if !enabled
        .iter()
        .any(|key| key == flags::ORG_AGENT_ACCESS_FLAG_KEY)
    {
        return Ok(false);
    }
    match security_readiness_service::require_serving(db, flags::ORG_AGENT_ACCESS_FLAG_KEY).await {
        Ok(()) => Ok(true),
        Err(AppError::SecurityReadinessUnavailable) => Ok(false),
        Err(error) => Err(error),
    }
}

const MAX_SUBJECTS: usize = 128;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    // The explicit deserializer makes absence a schema error while accepting JSON null.
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    maintainers: Option<Vec<String>>,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    users: Option<Vec<String>>,
    expected_revision: i64,
}

impl Replacement {
    fn normalize(&mut self) -> AppResult<()> {
        if self.expected_revision < 0 {
            return Err(AppError::ValidationError(
                "expected_revision must be nonnegative".into(),
            ));
        }
        for ids in [&mut self.maintainers, &mut self.users]
            .into_iter()
            .flatten()
        {
            if ids.len() > MAX_SUBJECTS {
                return Err(AppError::ValidationError(
                    "Each list accepts at most 128 people".into(),
                ));
            }
            for id in ids.iter_mut() {
                *id = uuid::Uuid::parse_str(id)
                    .map_err(|_| AppError::ValidationError("Expected a person UUID".into()))?
                    .to_string();
            }
            ids.sort_unstable();
            if ids.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(AppError::ValidationError(
                    "Each person may appear only once per list".into(),
                ));
            }
        }
        Ok(())
    }
}

struct ListChange {
    added: usize,
    removed: usize,
    inherited_before: bool,
    inherited_after: bool,
    self_added: bool,
}

impl ListChange {
    fn between(before: &Option<Vec<String>>, after: &Option<Vec<String>>, actor: &str) -> Self {
        let prior: BTreeSet<_> = before.iter().flatten().map(String::as_str).collect();
        let next: BTreeSet<_> = after.iter().flatten().map(String::as_str).collect();
        Self {
            added: next.difference(&prior).count(),
            removed: prior.difference(&next).count(),
            inherited_before: before.is_none(),
            inherited_after: after.is_none(),
            self_added: next.contains(actor) && !prior.contains(actor),
        }
    }
    fn changed(&self) -> bool {
        self.inherited_after != self.inherited_before || self.added > 0 || self.removed > 0
    }
    fn narrowed(&self) -> bool {
        !self.inherited_after && (self.inherited_before || self.removed > 0)
    }
    fn audit(&self) -> serde_json::Value {
        json!({"added":self.added,"removed":self.removed,"inherited_before":self.inherited_before,
            "inherited_after":self.inherited_after,"self_added":self.self_added})
    }
}

async fn validate_subjects(
    db: &Database,
    tx: &mut ClientSession,
    org: &str,
    policy: &Replacement,
    fence: bool,
) -> AppResult<()> {
    let ids: BTreeSet<_> = policy
        .users
        .iter()
        .flatten()
        .chain(policy.maintainers.iter().flatten())
        .collect();
    let ids: Vec<_> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(());
    }
    for (collection, filter) in [
        (
            USERS,
            doc! {"_id":{"$in":&ids},"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}],"is_active":true,"platform_personal_deleting":{"$ne":true}},
        ),
        (
            MEMBERS,
            doc! {"org_user_id":org,"member_user_id":{"$in":&ids},"revoked_at":null,"role":{"$in":["admin","member"]}},
        ),
    ] {
        let rows = db.collection::<Document>(collection);
        let count = if fence {
            rows.update_many(filter, doc! {"$inc":{"org_channel_fence":1_i64}})
                .session(&mut *tx)
                .await?
                .matched_count
        } else {
            rows.count_documents(filter).session(&mut *tx).await?
        };
        if count != ids.len() as u64 {
            return Err(AppError::ValidationError(
                "Select active Admins or Members of this organization".into(),
            ));
        }
    }
    Ok(())
}

fn stale() -> AppError {
    AppError::Conflict(
        "Specialist access policy changed. Read it again before applying edits.".into(),
    )
}

pub(crate) async fn replace(
    db: &Database,
    actor: AuditActor,
    original: AssistantAgent,
    mut policy: Replacement,
) -> AppResult<AssistantAgent> {
    policy.normalize()?;
    if original.org_access_revision != policy.expected_revision {
        return Err(stale());
    }
    let maintain = ListChange::between(
        &original.org_maintainer_user_ids,
        &policy.maintainers,
        &actor.user_id,
    );
    let usage = ListChange::between(&original.org_user_ids, &policy.users, &actor.user_id);
    let intent = if maintain.changed() || usage.changed() {
        Some(required::intent(
            Some(&actor),
            "org_agent_access_changed",
            Some(json!({
                "org_user_id":original.user_id,"agent_id":original.id,
                "revision":policy.expected_revision.checked_add(1).ok_or_else(|| AppError::Internal("Specialist access revision exhausted".into()))?,
                "maintainers":maintain.audit(),"users":usage.audit()
            })),
        )?)
    } else {
        None
    };
    let ctx = (
        db,
        actor.user_id,
        original.user_id,
        original.id,
        policy,
        maintain,
        usage,
        intent,
    );
    transaction_service::run_db_only(
        db,
        &ctx,
        |tx, (db, actor, org, id, policy, maintain, usage, intent)| {
            Box::pin(async move {
                let agents = db.collection::<AssistantAgent>(AGENTS);
                let filter = live_specialist_filter(org, id);
                let current = agents
                    .find_one(filter.clone())
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound("Organization specialist not found".into())
                    })?;
                let fence = if intent.is_some() {
                    MemberFence::MemberAndOrg
                } else {
                    MemberFence::ReadOnly
                };
                if !member_rights_in_session(db, tx, actor, &current, fence)
                    .await?
                    .can_govern
                {
                    return Err(AppError::Forbidden(
                        "Organization administrator access is required".into(),
                    ));
                }
                if !available(db, actor, org).await? {
                    return Err(AppError::OrgAgentAccessAuthoringUnavailable);
                }
                if current.org_access_revision != policy.expected_revision {
                    return Err(stale());
                }
                validate_subjects(db, tx, org, policy, intent.is_some()).await?;
                let Some(intent) = intent else {
                    return Ok(current);
                };
                // Concurrent changes to the specialist abort this snapshot and retry the revision check.
                let update = doc! {
                    "$set": {
                        "org_maintainer_user_ids": &policy.maintainers,
                        "org_user_ids": &policy.users,
                        "updated_at": bson::DateTime::now()
                    },
                    "$inc": {
                        "org_access_revision": 1_i64,
                        "org_maintain_revision": i64::from(maintain.narrowed()),
                        "org_use_revision": i64::from(usage.narrowed())
                    }
                };
                let saved = agents
                    .find_one_and_update(filter, update)
                    .return_document(ReturnDocument::After)
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(stale)?;
                security_effect_service::enqueue_in_session(db, tx, std::slice::from_ref(intent))
                    .await?;
                Ok(saved)
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests;
