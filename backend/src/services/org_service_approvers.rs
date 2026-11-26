//! Live organization service approval authority. Delivery snapshots are routing hints.
use crate::errors::{AppError, AppResult};
use crate::models::{
    approval_request::ApprovalRequest,
    notification_channel::COLLECTION_NAME as CHANNELS,
    org_membership::{COLLECTION_NAME as MEMBERS, MemberScopeSource, OrgMembership},
    org_role_scope::{COLLECTION_NAME as SCOPES, OrgRoleScope},
    service_approval_config::{COLLECTION_NAME as CONFIGS, OrgPolicyStamp, ServiceApprovalConfig},
    user::COLLECTION_NAME as USERS,
    user_service::COLLECTION_NAME as SERVICES,
};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};

pub(crate) const FLAG: &str = "approval:org-service-named-approvers-v1";

pub(crate) fn refused() -> AppError {
    AppError::Forbidden("You are not an eligible approver for this service".into())
}

fn stale() -> AppError {
    AppError::Conflict("Organization approval policy changed".into())
}

pub(crate) fn no_admin() -> AppError {
    AppError::OrgApprovalNoAdmin(
        "No eligible administrator can decide this service approval".into(),
    )
}

pub(crate) fn stamp(cfg: &ServiceApprovalConfig, connection: Option<&str>) -> OrgPolicyStamp {
    OrgPolicyStamp {
        config_id: cfg.id.clone(),
        revision: cfg.approvers.approver_revision,
        connection_id: connection.map(str::to_owned),
    }
}

pub(crate) fn revision_live(
    cfg: &ServiceApprovalConfig,
    binding: Option<&OrgPolicyStamp>,
    pending: bool,
) -> bool {
    let min = if pending {
        cfg.approvers.approver_revision
    } else {
        cfg.approvers.min_grant_revision
    };
    match binding {
        Some(s) => {
            s.config_id == cfg.id && (min..=cfg.approvers.approver_revision).contains(&s.revision)
        }
        None => min == 0,
    }
}

pub(crate) fn connection(request: &ApprovalRequest) -> Option<&str> {
    request
        .org_policy
        .as_ref()
        .and_then(|s| s.connection_id.as_deref())
        .or_else(|| {
            request
                .exact_service
                .as_ref()
                .map(|b| b.user_service_id.as_str())
        })
}

pub(crate) async fn config(
    db: &Database,
    org: &str,
    service: &str,
    tx: &mut ClientSession,
) -> AppResult<Option<ServiceApprovalConfig>> {
    Ok(db
        .collection::<ServiceApprovalConfig>(CONFIGS)
        .find_one(doc! {"user_id":org,"service_id":service})
        .session(tx)
        .await?)
}

pub(crate) async fn fence_config(
    db: &Database,
    cfg: &ServiceApprovalConfig,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let result = db.collection::<Document>(CONFIGS).update_one(doc! {"_id": &cfg.id, "$expr":{"$eq":[{"$ifNull":["$approver_revision",0]},cfg.approvers.approver_revision]}},
        doc! {"$inc":{"approval_decision_fence":1_i64}}).session(tx).await?;
    if result.matched_count != 1 {
        return Err(stale());
    }
    Ok(())
}

fn live_person() -> Document {
    doc! {"is_active":true,"user_type":{"$ne":"org"},"platform_personal_deleting":{"$ne":true}}
}

pub(crate) async fn eligible(
    db: &Database,
    cfg: &ServiceApprovalConfig,
    connection: Option<&str>,
    actor: Option<&str>,
    fence: bool,
    tx: &mut ClientSession,
) -> AppResult<Vec<String>> {
    let org = &cfg.user_id;
    if db.collection::<Document>(USERS).find_one(doc! {"_id":org,"user_type":"org","is_active":true,"platform_personal_deleting":{"$ne":true}}).projection(doc! {"_id":1}).session(&mut *tx).await?.is_none() {return Ok(vec![]);}
    let mut filter = doc! {"org_user_id":org,"role":"admin","revoked_at":null};
    if let Some(ids) = &cfg.approvers.approver_user_ids {
        filter.insert("member_user_id", doc! {"$in":ids});
    }
    if let Some(actor) = actor {
        if cfg
            .approvers
            .approver_user_ids
            .as_ref()
            .is_some_and(|ids| !ids.iter().any(|id| id == actor))
        {
            return Ok(Vec::new());
        }
        filter.insert("member_user_id", actor);
    }
    let mut members = db
        .collection::<OrgMembership>(MEMBERS)
        .find(filter)
        .session(&mut *tx)
        .await?;
    let members: Vec<_> = members.stream(&mut *tx).try_collect().await?;
    if members.is_empty() {
        return Ok(Vec::new());
    }

    let inherited = if members
        .iter()
        .any(|m| m.scope_source == MemberScopeSource::Inherit)
    {
        db.collection::<OrgRoleScope>(SCOPES)
            .find_one(doc! {"org_user_id":org,"role":"admin"})
            .session(&mut *tx)
            .await?
            .and_then(|s| s.allowed_service_ids)
    } else {
        None
    };
    let mut targets = Vec::new();
    if connection.is_some()
        || members.iter().any(|m| match m.scope_source {
            MemberScopeSource::Inherit => inherited.is_some(),
            MemberScopeSource::Override => m.allowed_service_ids.is_some(),
        })
    {
        let mut target = doc! {"user_id":org,"deleted_at":null,"is_active":true,"$or":[{"_id":&cfg.service_id},{"catalog_service_id":&cfg.service_id}]};
        if let Some(id) = connection {
            target.insert("_id", id);
        }
        let mut cursor = db
            .collection::<Document>(SERVICES)
            .find(target)
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?;
        while let Some(row) = cursor.next(&mut *tx).await.transpose()? {
            if let Ok(id) = row.get_str("_id") {
                targets.push(id.to_owned());
            }
        }
        if connection.is_some() && targets.is_empty() {
            return Ok(vec![]);
        }
    }
    let mut candidates = Vec::new();
    for m in members {
        let allowed = if m.scope_source == MemberScopeSource::Override {
            m.allowed_service_ids.as_deref()
        } else {
            inherited.as_deref()
        };
        // Without a concrete connection a scoped Admin must cover every possible sibling.
        if allowed.is_none_or(|ids| !targets.is_empty() && targets.iter().all(|s| ids.contains(s)))
        {
            candidates.push(m.member_user_id);
        }
    }
    let mut people = live_person();
    people.insert("_id", doc! {"$in": &candidates});
    let mut cursor = db
        .collection::<Document>(USERS)
        .find(people)
        .projection(doc! {"_id":1})
        .sort(doc! {"_id":1})
        .session(&mut *tx)
        .await?;
    let mut ids = Vec::new();
    while let Some(row) = cursor.next(&mut *tx).await.transpose()? {
        if let Ok(id) = row.get_str("_id") {
            ids.push(id.to_owned());
        }
    }
    if fence && !ids.is_empty() {
        // The org fence orders membership changes and inherited role-scope creation
        // (including the previously absent default). Each selected person and
        // connection conflicts with concurrent disable, unlink or removal.
        super::org_membership_authority::fence_org(db, org, tx).await?;
        for id in &ids {
            super::org_membership_authority::fence_live_member(db, id, tx).await?;
        }
        if !targets.is_empty() {
            db.collection::<Document>(SERVICES)
                .update_many(
                    doc! {"_id":{"$in":targets}},
                    doc! {"$inc":{"approval_decision_fence":1_i64}},
                )
                .session(tx)
                .await?;
        }
    }
    Ok(ids)
}

pub(crate) async fn authorize(
    db: &Database,
    request: &ApprovalRequest,
    actor: &str,
    tx: &mut ClientSession,
) -> AppResult<ServiceApprovalConfig> {
    let cfg = config(db, &request.user_id, &request.service_id, tx)
        .await?
        .ok_or_else(refused)?;
    if request
        .assistant_group
        .as_ref()
        .is_some_and(|g| g.actor_user_id != actor)
        || eligible(db, &cfg, connection(request), Some(actor), true, tx)
            .await?
            .is_empty()
    {
        return Err(refused());
    }
    if !revision_live(&cfg, request.org_policy.as_ref(), true) {
        return Err(stale());
    }
    fence_config(db, &cfg, tx).await?;
    Ok(cfg)
}

pub(crate) async fn visible(
    db: &Database,
    request: &ApprovalRequest,
    actor: &str,
) -> AppResult<bool> {
    if !request.from_org_policy {
        return Ok(true);
    }
    let scope = history_filter(db, actor, Some(&request.user_id), false).await?;
    Ok(db
        .collection::<Document>(crate::models::approval_request::COLLECTION_NAME)
        .find_one(doc! {"_id":&request.id,"$and":[scope]})
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

pub(crate) async fn recipients(db: &Database, request: &ApprovalRequest) -> AppResult<Vec<String>> {
    if !request.from_org_policy {
        return Ok(if request.notify_user_ids.is_empty() {
            vec![request.user_id.clone()]
        } else {
            request.notify_user_ids.clone()
        });
    }
    let mut session = db.client().start_session().await?;
    let Some(cfg) = config(db, &request.user_id, &request.service_id, &mut session).await? else {
        return Ok(vec![]);
    };
    if !revision_live(&cfg, request.org_policy.as_ref(), true) {
        return Ok(vec![]);
    }
    let mut ids = eligible(
        db,
        &cfg,
        connection(request),
        request
            .assistant_group
            .as_ref()
            .map(|g| g.actor_user_id.as_str()),
        false,
        &mut session,
    )
    .await?;
    ids.retain(|id| request.notify_user_ids.contains(id));
    Ok(ids)
}

pub(crate) async fn telegram_actor(
    db: &Database,
    sender: i64,
    tx: &mut ClientSession,
) -> AppResult<String> {
    let channels = db.collection::<Document>(CHANNELS);
    let mut cursor = channels
        .find(doc! {"telegram_chat_id":sender})
        .projection(doc! {"user_id":1,"telegram_enabled":1})
        .limit(2)
        .session(&mut *tx)
        .await?;
    let links: Vec<_> = cursor.stream(&mut *tx).try_collect().await?;
    let [link] = links.as_slice() else {
        return Err(refused());
    };
    if link.get_bool("telegram_enabled") != Ok(true) {
        return Err(refused());
    }
    let actor = link.get_str("user_id").map_err(|_| refused())?;
    channels
        .update_one(
            doc! {"user_id":actor,"telegram_chat_id":sender,"telegram_enabled":true},
            doc! {"$inc":{"approval_decision_fence":1_i64}},
        )
        .session(tx)
        .await?;
    Ok(actor.to_owned())
}

/// Bind legacy evidence to the retired UUID before the service key can be reused.
pub(crate) async fn delete_config_in_session(
    db: &Database,
    owner_service: &Document,
    tx: &mut ClientSession,
) -> mongodb::error::Result<u64> {
    use crate::models::{
        approval_grant::COLLECTION_NAME as GRANTS, approval_request::COLLECTION_NAME as REQUESTS,
    };
    let Some(cfg) = db
        .collection::<ServiceApprovalConfig>(CONFIGS)
        .find_one_and_delete(owner_service.clone())
        .session(&mut *tx)
        .await?
    else {
        return Ok(0);
    };
    let mut legacy = owner_service.clone();
    legacy.extend(doc! {"from_org_policy":true,"org_policy":null,"status":"approved"});
    db.collection::<Document>(REQUESTS)
        .update_many(
            legacy,
            doc! {"$set":{"org_policy":{
                "config_id":&cfg.id,"revision":cfg.approvers.approver_revision
            }}},
        )
        .session(&mut *tx)
        .await?;
    let mut pending = owner_service.clone();
    pending.extend(doc! {"execution_proposal":null,"status":"pending",
    "$or":[{"from_org_policy":true},{"approval_mode":{"$in":["grant",null]}}]});
    db.collection::<Document>(REQUESTS)
        .update_many(
            pending,
            doc! {"$set":{"status":"expired","decided_at":bson::DateTime::now()}},
        )
        .session(&mut *tx)
        .await?;
    let mut grants = owner_service.clone();
    grants.extend(doc! {"revoked":false,"execution":null});
    db.collection::<Document>(GRANTS)
        .update_many(grants, doc! {"$set":{"revoked":true}})
        .session(tx)
        .await?;
    Ok(1)
}

pub(crate) fn member_list_filter(actor: &str) -> Document {
    doc! {"approver_user_ids":actor}
}

pub(crate) fn prune_update(actor: &str) -> Vec<Document> {
    let next = doc! {"$add":[{"$ifNull":["$approver_revision",0_i64]},1_i64]};
    vec![
        doc! {"$set":{"approver_revision": &next,"min_grant_revision":next,"approver_grants_pending":true,
        "approver_user_ids":{"$filter":{"input":"$approver_user_ids","cond":{"$ne":["$$this",{"$literal":actor}]}}}}},
    ]
}

pub(crate) async fn prune(
    db: &Database,
    org: &str,
    actor: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(CONFIGS)
        .update_many(
            doc! {"user_id":org,"approver_user_ids":actor},
            prune_update(actor),
        )
        .session(tx)
        .await?;
    Ok(())
}

pub(crate) async fn request_live(db: &Database, request: &ApprovalRequest) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    live_in_session(db, request, false, &mut session).await
}

pub(crate) async fn live_in_session(
    db: &Database,
    request: &ApprovalRequest,
    fence: bool,
    tx: &mut ClientSession,
) -> AppResult<bool> {
    let Some(cfg) = config(db, &request.user_id, &request.service_id, tx).await? else {
        return Ok(false);
    };
    if !revision_live(
        &cfg,
        request.org_policy.as_ref(),
        request.status == "pending",
    ) {
        return Ok(false);
    }
    if cfg.approvers.approver_user_ids.is_some()
        && eligible(
            db,
            &cfg,
            connection(request),
            request
                .assistant_group
                .as_ref()
                .map(|g| g.actor_user_id.as_str()),
            false,
            tx,
        )
        .await?
        .is_empty()
    {
        return Ok(false);
    }
    if fence {
        fence_config(db, &cfg, tx).await?;
    }
    Ok(true)
}

pub(crate) async fn insert_request(db: &Database, request: &ApprovalRequest) -> AppResult<()> {
    super::transaction_service::run_db_only(db, &(db, request), |tx, (db, r)| {
        Box::pin(async move {
            let cfg = config(db, &r.user_id, &r.service_id, tx)
                .await?
                .ok_or_else(stale)?;
            if !revision_live(&cfg, r.org_policy.as_ref(), true) {
                return Err(stale());
            }
            fence_config(db, &cfg, tx).await?;
            let current = eligible(
                db,
                &cfg,
                connection(r),
                r.assistant_group.as_ref().map(|g| g.actor_user_id.as_str()),
                false,
                tx,
            )
            .await?;
            if !current.iter().any(|id| r.notify_user_ids.contains(id)) {
                return Err(no_admin());
            }
            super::erasure_admission::fence_in_session(db, &r.user_id, tx).await?;
            db.collection::<ApprovalRequest>(crate::models::approval_request::COLLECTION_NAME)
                .insert_one(*r)
                .session(tx)
                .await?;
            Ok(())
        })
    })
    .await
}

/// One request-local read of the actor's Admin scopes. A retired policy or connection
/// remains part of history. A live named policy restricts rows stamped with its UUID.
pub(crate) struct HistoryAccess {
    pub filter: Document,
    pub orgs: Vec<super::approval_service::OrgFilterBranch>,
    pub configs: Vec<ServiceApprovalConfig>,
}

impl HistoryAccess {
    pub(crate) async fn load(
        db: &Database,
        actor: &str,
        org: Option<&str>,
        grants: bool,
    ) -> AppResult<Self> {
        let mut membership = doc! {"member_user_id":actor,"role":"admin","revoked_at":null};
        if let Some(org) = org {
            membership.insert("org_user_id", org);
        }
        let members: Vec<_> = db
            .collection::<OrgMembership>(MEMBERS)
            .find(membership)
            .await?
            .try_collect()
            .await?;
        let org_ids: Vec<_> = members.iter().map(|m| &m.org_user_id).collect();
        let mut personal = live_person();
        personal.insert("_id", actor);
        let mut branches = Vec::new();
        if org.is_none() {
            branches.push(
                doc! {"user_id":actor,"from_org_policy":{"$ne":true},"org_scoped":{"$ne":true}},
            );
        }
        let mut orgs = Vec::new();
        let mut config_owners = Vec::new();
        if grants && org.is_none() {
            config_owners.push(actor.to_owned());
        }
        let owners: Vec<Document> = if members.is_empty() {
            Vec::new()
        } else {
            let orgs = doc! {"_id":{"$in":&org_ids},"user_type":"org","is_active":true,"platform_personal_deleting":{"$ne":true}};
            db.collection(USERS)
                .find(doc! {"$or":[personal,orgs]})
                .projection(doc! {"user_type":1})
                .await?
                .try_collect()
                .await?
        };
        let actor_live = owners
            .iter()
            .any(|row| row.get_str("_id") == Ok(actor) && row.get_str("user_type") != Ok("org"));
        let live_orgs: std::collections::HashSet<_> = owners
            .iter()
            .filter(|row| row.get_str("user_type") == Ok("org"))
            .filter_map(|row| row.get_str("_id").ok())
            .collect();
        let members: Vec<_> = members
            .into_iter()
            .filter(|m| actor_live && live_orgs.contains(m.org_user_id.as_str()))
            .collect();
        let inherited: Vec<_> = members
            .iter()
            .filter(|m| m.scope_source == MemberScopeSource::Inherit)
            .map(|m| &m.org_user_id)
            .collect();
        let defaults: Vec<OrgRoleScope> = if inherited.is_empty() {
            Vec::new()
        } else {
            db.collection(SCOPES)
                .find(doc! {"org_user_id":{"$in":inherited},"role":"admin"})
                .await?
                .try_collect()
                .await?
        };
        let mut scopes = Vec::new();
        let mut scoped_connections = Vec::new();
        for m in members {
            let ids = match m.scope_source {
                MemberScopeSource::Override => m.allowed_service_ids,
                MemberScopeSource::Inherit => defaults
                    .iter()
                    .find(|s| s.org_user_id == m.org_user_id)
                    .and_then(|s| s.allowed_service_ids.clone()),
            };
            if !grants || ids.is_some() {
                let mut owner = doc! {"user_id":&m.org_user_id};
                if let Some(ids) = &ids {
                    owner.insert("_id", doc! {"$in":ids});
                }
                scoped_connections.push(owner);
            }
            config_owners.push(m.org_user_id.clone());
            scopes.push((m.org_user_id, ids));
        }
        let connections: Vec<Document> = if scoped_connections.is_empty() {
            Vec::new()
        } else {
            db.collection(SERVICES)
                .find(doc! {"$or":scoped_connections})
                .projection(doc! {"user_id":1,"catalog_service_id":1,"is_active":1,"deleted_at":1})
                .await?
                .try_collect()
                .await?
        };
        let configs: Vec<ServiceApprovalConfig> = if config_owners.is_empty() {
            Vec::new()
        } else {
            db.collection(CONFIGS)
                .find(doc! {"user_id":{"$in":config_owners}})
                .await?
                .try_collect()
                .await?
        };
        for (owner, ids) in scopes {
            let mut branch = doc! {"user_id":&owner};
            if !grants {
                branch.insert(
                    "$or",
                    vec![
                        doc! {"assistant_group":null},
                        doc! {"assistant_group.actor_user_id":actor},
                    ],
                );
            }
            let excluded: Vec<_> = configs
                .iter()
                .filter(|c| {
                    c.user_id == owner
                        && c.approvers
                            .approver_user_ids
                            .as_ref()
                            .is_some_and(|ids| !ids.iter().any(|id| id == actor))
                })
                .map(|c| &c.id)
                .collect();
            let mut conditions = Vec::new();
            if !excluded.is_empty() {
                let exception = if grants {
                    doc! {"execution":{"$type":"object"}}
                } else {
                    doc! {"from_org_policy":{"$ne":true}}
                };
                conditions
                    .push(doc! {"$or":[exception,{"org_policy.config_id":{"$nin":&excluded}}]});
            }
            if let Some(ids) = &ids {
                let mut legacy = ids.clone();
                legacy.extend(
                    connections
                        .iter()
                        .filter(|r| r.get_str("user_id") == Ok(&owner))
                        .filter_map(|r| r.get_str("catalog_service_id").ok().map(str::to_owned)),
                );
                conditions.push(doc!{"$or":[{"org_policy.connection_id":{"$in":ids}},
                    {"org_policy.connection_id":null,"exact_service.user_service_id":{"$in":ids}},
                    {"org_policy.connection_id":null,"exact_service.user_service_id":null,"service_id":{"$in":legacy}}]});
            }
            if !grants {
                // Historical visibility never grants authority on an outstanding prompt.
                let eligible: Vec<_> = configs
                    .iter()
                    .filter(|cfg| cfg.user_id == owner && !excluded.contains(&&cfg.id))
                    .collect();
                let mut versions:Vec<Document>=eligible.iter().map(|cfg|doc!{"org_policy.config_id":&cfg.id,"org_policy.revision":cfg.approvers.approver_revision}).collect();
                let legacy_configs: Vec<_> = eligible
                    .iter()
                    .filter(|cfg| cfg.approvers.approver_revision == 0)
                    .map(|cfg| &cfg.service_id)
                    .collect();
                versions.push(doc! {"org_policy":null,"service_id":{"$in":legacy_configs}});
                let live: Vec<_> = connections
                    .iter()
                    .filter(|c| {
                        c.get_str("user_id") == Ok(&owner)
                            && c.get_bool("is_active") == Ok(true)
                            && c.get_datetime("deleted_at").is_err()
                    })
                    .collect();
                let live_ids: Vec<_> = live.iter().filter_map(|c| c.get_str("_id").ok()).collect();
                let mut legacy_target =
                    doc! {"org_policy.connection_id":null,"exact_service.user_service_id":null};
                if ids.is_some() {
                    let mut keys = live_ids.clone();
                    keys.extend(
                        live.iter()
                            .filter_map(|c| c.get_str("catalog_service_id").ok()),
                    );
                    legacy_target.insert("service_id", doc! {"$in":keys});
                }
                let target = doc! {"$or":[legacy_target, {"org_policy.connection_id":{"$in":&live_ids}},
                {"org_policy.connection_id":null,"exact_service.user_service_id":{"$in":&live_ids}}]};
                conditions.push(
                    doc! {"$or":[{"from_org_policy":{"$ne":true}},{"status":{"$ne":"pending"}},
                    {"$and":[{"$or":versions},target]}]},
                );
            }
            if !conditions.is_empty() {
                branch.insert("$and", conditions);
            }
            branches.push(branch);
            // The complete scope is already in the Mongo predicate, including tombstones.
            orgs.push(super::approval_service::OrgFilterBranch {
                org_id: owner,
                service_id_scope: None,
            });
        }
        let filter = if branches.is_empty() {
            doc! {"_id":{"$in":[]}}
        } else {
            doc! {"$or":branches}
        };
        Ok(Self {
            filter,
            orgs,
            configs,
        })
    }
}

pub(crate) async fn history_filter(
    db: &Database,
    actor: &str,
    org: Option<&str>,
    grants: bool,
) -> AppResult<Document> {
    Ok(HistoryAccess::load(db, actor, org, grants).await?.filter)
}

/// Probe with the original indexed card query. Only an org-policy candidate needs
/// the additional predicate. Reapply the sort and limit in Mongo so hidden cards
/// cannot displace visible ones. Reuse the predicate across pages of the same call.
#[derive(Default)]
pub(crate) struct CardVisibility(Option<Document>);
impl CardVisibility {
    pub(crate) async fn find<T: serde::de::DeserializeOwned + Unpin + Send + Sync>(
        &mut self,
        db: &Database,
        actor: &str,
        base: Document,
        options: mongodb::options::FindOptions,
        from_org_policy: fn(&T) -> bool,
    ) -> AppResult<Vec<T>> {
        let collection = db.collection::<T>(crate::models::approval_request::COLLECTION_NAME);
        if self.0.is_none() {
            let candidates: Vec<T> = collection
                .find(base.clone())
                .with_options(options.clone())
                .await?
                .try_collect()
                .await?;
            if !candidates.iter().any(from_org_policy) {
                return Ok(candidates);
            }
            let orgs = history_filter(db, actor, None, false).await?;
            self.0 = Some(doc! {"$or":[{"from_org_policy":{"$ne":true}},orgs]});
        }
        let predicate = self.0.as_ref().expect("initialized for org-policy cards");
        Ok(collection
            .find(doc! {"$and":[base,predicate]})
            .with_options(options)
            .await?
            .try_collect()
            .await?)
    }
}

pub(crate) async fn can_edit_chat(
    db: &Database,
    request: &ApprovalRequest,
    chat: i64,
    recipients: &[String],
) -> AppResult<bool> {
    if !request.from_org_policy {
        return Ok(true);
    }
    if chat <= 0 {
        return Ok(false);
    }
    let links: Vec<_> = db
        .collection::<Document>(CHANNELS)
        .find(doc! {"telegram_chat_id":chat})
        .projection(doc! {"user_id":1,"telegram_enabled":1})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    Ok(
        matches!(links.as_slice(),[link] if link.get_bool("telegram_enabled")==Ok(true) && link.get_str("user_id").is_ok_and(|actor| recipients.iter().any(|id|id==actor))),
    )
}

/// Old and new writers can both have outstanding prompts during a rolling upgrade.
/// Current revisions are monotonically increasing, so a stale pending ID cannot become live again.
pub(crate) async fn stale_pending(
    db: &Database,
    now: bson::DateTime,
    limit: usize,
) -> AppResult<Vec<ApprovalRequest>> {
    let pipeline = [
        doc! {"$match":{"status":"pending","from_org_policy":true,"expires_at":{"$gt":now}}},
        doc! {"$lookup":{"from":CONFIGS,"let":{"owner":"$user_id","svc":"$service_id"},"pipeline":[
            {"$match":{"$expr":{"$and":[{"$eq":["$user_id","$$owner"]},{"$eq":["$service_id","$$svc"]}]}}},
            {"$project":{"approver_revision":1}}
        ],"as":"policy"}},
        doc! {"$unwind":{"path":"$policy","preserveNullAndEmptyArrays":true}},
        doc! {"$match":{"$or":[{"policy":null},{"$expr":{"$ne":[{"$ifNull":["$org_policy.revision",0]},{"$ifNull":["$policy.approver_revision",0]}]}},
        {"org_policy.config_id":{"$type":"string"},"$expr":{"$ne":["$org_policy.config_id","$policy._id"]}}]}},
        doc! {"$limit":limit as i64},
        doc! {"$unset":"policy"},
    ];
    Ok(db
        .collection::<ApprovalRequest>(crate::models::approval_request::COLLECTION_NAME)
        .aggregate(pipeline)
        .with_type::<ApprovalRequest>()
        .await?
        .try_collect()
        .await?)
}

#[cfg(test)]
mod tests;

/// Recover the policy stamp of a grant written by an older replica that had
/// read a stamped request. A failed join cannot supply missing authority.
pub(crate) fn grant_origin_lookup() -> Document {
    doc! {"$lookup":{"from":crate::models::approval_request::COLLECTION_NAME,
    "localField":"approval_request_id","foreignField":"_id",
    "let":{"org":"$user_id","svc":"$service_id","kind":"$requester_type","requester":"$requester_id"},
    "pipeline":[{"$match":{"from_org_policy":true,"status":"approved","$expr":{"$and":[
        {"$eq":["$user_id","$$org"]},{"$eq":["$service_id","$$svc"]},
        {"$eq":["$requester_type","$$kind"]},{"$eq":["$requester_id","$$requester"]}
    ]}}},{"$project":{"_id":1,"org_policy":1}}],"as":"origin"}}
}

pub(crate) fn effective_stamp() -> Document {
    doc! {"$set":{"org_policy":{"$ifNull":["$org_policy",{"$first":"$origin.org_policy"}]}}}
}

/// Restrict active-grant pagination to grants still reusable under the saved cutoff.
pub(crate) async fn list_policy_grants(
    db: &Database,
    filter: Document,
    visibility: Document,
    narrowed: &[&ServiceApprovalConfig],
    offset: u64,
    limit: u64,
) -> AppResult<(Vec<crate::models::approval_grant::ApprovalGrant>, u64)> {
    let mut predicates = vec![visibility];
    for cfg in narrowed {
        predicates.push(doc! {"$or":[{"user_id":{"$ne":&cfg.user_id}},{"service_id":{"$ne":&cfg.service_id}},
            {"execution":{"$type":"object"}},{"org_policy.config_id":&cfg.id,"org_policy.revision":{"$gte":cfg.approvers.min_grant_revision,"$lte":cfg.approvers.approver_revision}}]});
    }
    let mut items = vec![
        doc! {"$sort":{"granted_at":-1,"_id":1}},
        doc! {"$skip":i64::try_from(offset).unwrap_or(i64::MAX)},
    ];
    if limit > 0 {
        items.push(doc! {"$limit":i64::try_from(limit).unwrap_or(100)});
    }
    items.push(doc! {"$unset":"origin"});
    #[derive(serde::Deserialize)]
    struct Count {
        total: u64,
    }
    #[derive(serde::Deserialize)]
    struct Page {
        items: Vec<crate::models::approval_grant::ApprovalGrant>,
        counts: Vec<Count>,
    }
    let page = db
        .collection::<Document>(crate::models::approval_grant::COLLECTION_NAME)
        .aggregate([
            doc! {"$match":filter},
            grant_origin_lookup(),
            effective_stamp(),
            doc! {"$match":{"$and":predicates}},
            doc! {"$facet":{"items":items,"counts":[{"$count":"total"}]}},
        ])
        .with_type::<Page>()
        .await?
        .try_next()
        .await?;
    Ok(page
        .map(|p| (p.items, p.counts.first().map_or(0, |c| c.total)))
        .unwrap_or_default())
}

/// Narrowing writes this marker alongside the cutoff. Reads already enforce
/// the cutoff. Each sweep retires at most one page of proven policy grants.
pub(crate) async fn retire_grants(db: &Database) -> AppResult<()> {
    use crate::models::approval_grant::COLLECTION_NAME as GRANTS;
    let configs = db.collection::<ServiceApprovalConfig>(CONFIGS);
    let Some(cfg) = configs
        .find_one(doc! {"approver_grants_pending":true})
        .await?
    else {
        return Ok(());
    };
    let scope =
        doc! {"user_id":&cfg.user_id,"service_id":&cfg.service_id,"execution":null,"revoked":false};
    let mut stale = doc! {"$or":[{"org_policy.config_id":{"$ne":&cfg.id}},
    {"org_policy.revision":{"$not":{"$gte":cfg.approvers.min_grant_revision}}}]};
    stale.insert("$and",[doc! {"$or":[{"org_scoped":true},{"org_policy":{"$type":"object"}},{"origin.0":{"$exists":true}}]}].to_vec());
    let ids: Vec<Document> = db
        .collection::<Document>(GRANTS)
        .aggregate([
            doc! {"$match":scope},
            grant_origin_lookup(),
            effective_stamp(),
            doc! {"$match":stale},
            doc! {"$limit":100},
            doc! {"$project":{"_id":1}},
        ])
        .await?
        .try_collect()
        .await?;
    if ids.is_empty() {
        configs
            .update_one(
                doc! {"_id":&cfg.id,"min_grant_revision":cfg.approvers.min_grant_revision},
                doc! {"$unset":{"approver_grants_pending":""}},
            )
            .await?;
    } else {
        let ids: Vec<_> = ids.iter().filter_map(|r| r.get_str("_id").ok()).collect();
        db.collection::<Document>(GRANTS)
            .update_many(
                doc! {"_id":{"$in":ids},"execution":null,"revoked":false},
                doc! {"$set":{"revoked":true}},
            )
            .await?;
    }
    Ok(())
}
