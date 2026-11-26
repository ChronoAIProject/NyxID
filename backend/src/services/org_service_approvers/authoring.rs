//! Human administration of a shared org service approval configuration.
use super::{AdminSelection, AdminTarget, CONFIGS, FLAG, USERS, config, select_admins, stale};
use crate::models::{
    security_effect_intent::SecurityEffectIntent, service_approval_config::ServiceApprovalConfig,
};
use crate::{
    errors::{AppError, AppResult},
    services::{
        audit_service::{AuditActor, required},
        feature_flag_service,
        org_access_list::ListChange,
        security_effect_service, transaction_service,
    },
};
use bson::doc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::OnceCell;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Replacement {
    config_id: Uuid,
    expected_revision: i64,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    approver_user_ids: Option<Vec<Uuid>>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Candidate {
    #[serde(rename(deserialize = "_id"))]
    user_id: String,
    display_name: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Policy {
    config_id: String,
    revision: i64,
    approver_user_ids: Option<Vec<String>>,
    authoring_available: bool,
    eligible_candidates: Vec<Candidate>,
}

impl Replacement {
    fn normalized(&self) -> AppResult<Option<Vec<String>>> {
        if self.expected_revision < 0 {
            return Err(AppError::ValidationError(
                "Expected a nonnegative revision".into(),
            ));
        }
        self.approver_user_ids
            .as_ref()
            .map(|ids| {
                if !(1..=128).contains(&ids.len()) {
                    return Err(AppError::ValidationError(
                        "Select 1 to 128 approvers or reset to inheritance".into(),
                    ));
                }
                let mut ids: Vec<_> = ids.iter().map(Uuid::to_string).collect();
                ids.sort_unstable();
                if ids.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Err(AppError::ValidationError(
                        "Select each approver only once".into(),
                    ));
                }
                Ok(ids)
            })
            .transpose()
    }
}

/// A concrete connection must be in the editor's scope. A catalog config can
/// be managed through any in-scope sibling, independently of its decision list.
pub(crate) async fn policy(
    db: &Database,
    actor: AuditActor,
    org: &str,
    service: &str,
    connection: Option<&str>,
    replacement: Option<Replacement>,
) -> AppResult<Policy> {
    let access = Access {
        db,
        actor,
        org,
        service,
        connection,
        replacement,
        audit: OnceCell::new(),
    };
    transaction_service::run_db_only(db, &access, |tx, access| Box::pin(access.in_session(tx)))
        .await
}

struct Access<'a> {
    db: &'a Database,
    actor: AuditActor,
    org: &'a str,
    service: &'a str,
    connection: Option<&'a str>,
    replacement: Option<Replacement>,
    audit: OnceCell<SecurityEffectIntent>,
}

impl Access<'_> {
    async fn in_session(&self, tx: &mut ClientSession) -> AppResult<Policy> {
        let db = self.db;
        let current = config(db, self.org, self.service, tx).await?;
        let requested = self
            .replacement
            .as_ref()
            .map(Replacement::normalized)
            .transpose()?;
        let change = current.as_ref().zip(requested.as_ref()).map(|(cfg, ids)| {
            ListChange::between(&cfg.approvers.approver_user_ids, ids, &self.actor.user_id)
        });
        let changing = change.as_ref().is_some_and(ListChange::changed);
        let candidates = AdminSelection {
            org: self.org,
            service: self.service,
            target: AdminTarget::AnySibling,
            subjects: None,
        };
        let manager = AdminSelection {
            target: self
                .connection
                .map_or(AdminTarget::AnySibling, AdminTarget::Connection),
            ..candidates
        };
        if select_admins(db, manager, Some(&self.actor.user_id), changing, tx)
            .await?
            .is_empty()
        {
            return Err(AppError::OrgRoleInsufficient(
                "An administrator entitled to this service is required".into(),
            ));
        }
        let available = feature_flag_service::org_admin_authoring_available(
            db,
            &self.actor.user_id,
            self.org,
            FLAG,
        )
        .await?;
        if self.replacement.is_some() && !available {
            return Err(AppError::Forbidden(
                "Organization service approver authoring is unavailable".into(),
            ));
        }
        let mut cfg = current.ok_or_else(|| {
            if self.replacement.is_some() {
                stale()
            } else {
                AppError::NotFound("Organization service approval config not found".into())
            }
        })?;
        if let Some(edit) = &self.replacement {
            if edit.config_id.to_string() != cfg.id
                || edit.expected_revision != cfg.approvers.approver_revision
            {
                return Err(stale());
            }
            if let Some(Some(ids)) = &requested {
                let selection = AdminSelection {
                    subjects: Some(ids),
                    ..candidates
                };
                if select_admins(db, selection, None, changing, tx)
                    .await?
                    .len()
                    != ids.len()
                {
                    return Err(AppError::ValidationError(
                        "Select live organization Admins entitled to this service".into(),
                    ));
                }
            }
            if let (Some(ids), Some(change)) = (requested, change)
                && change.changed()
            {
                self.save(&mut cfg, ids, change, tx).await?;
            }
        }
        let ids = select_admins(db, candidates, None, false, tx).await?;
        let mut people = db
            .collection::<Candidate>(USERS)
            .find(doc! {"_id":{"$in":ids}})
            .sort(doc! {"_id":1})
            .projection(doc! {"_id":1,"display_name":1})
            .session(&mut *tx)
            .await?;
        let eligible_candidates = people.stream(tx).try_collect().await?;
        Ok(Policy {
            config_id: cfg.id,
            revision: cfg.approvers.approver_revision,
            approver_user_ids: cfg.approvers.approver_user_ids,
            authoring_available: available,
            eligible_candidates,
        })
    }

    async fn save(
        &self,
        cfg: &mut ServiceApprovalConfig,
        ids: Option<Vec<String>>,
        change: ListChange,
        tx: &mut ClientSession,
    ) -> AppResult<()> {
        let revision = cfg
            .approvers
            .approver_revision
            .checked_add(1)
            .ok_or_else(|| AppError::Internal("Approver revision exhausted".into()))?;
        let mut fields = doc! {"approver_revision":revision,"approver_user_ids":&ids,"updated_at":bson::DateTime::now()};
        if change.narrowed() {
            fields.extend(doc! {"min_grant_revision":revision,"approver_grants_pending":true});
        }
        self.db
            .collection::<bson::Document>(CONFIGS)
            .update_one(doc! {"_id":&cfg.id}, doc! {"$set":fields})
            .session(&mut *tx)
            .await?;
        // A retry must match the same config UUID and revision before reaching this audit.
        let audit = self.audit.get_or_try_init(|| async {
            required::intent(Some(&self.actor),"org_service_approvers_changed",Some(json!({
                "org_user_id":self.org,"service_id":self.service,"config_id":cfg.id,"revision":revision,
                "approvers":change.audit()
            })))
        }).await?;
        security_effect_service::enqueue_in_session(self.db, tx, std::slice::from_ref(audit))
            .await?;
        cfg.approvers.approver_revision = revision;
        cfg.approvers.approver_user_ids = ids;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
