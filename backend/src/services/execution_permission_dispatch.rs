//! Opt-in HTTP execution admission. Enrollment is a read-budget hint, never authority.
//! Gaps retain the legacy prompt chain. No permission authoring or proposal cards.
use std::sync::{Arc, OnceLock};

use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde_json::json;
use tokio::sync::OnceCell;
use uuid::Uuid;

use super::{
    assistant_acknowledgement_service as acks, execution_permission as contract,
    proxy_authorization::CanonicalPath, proxy_service,
};
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{approval_grant::ApprovalGrant, execution_permission::*},
};

mod authority;
pub(crate) mod tasks;

pub const READINESS: &str = "permissions:execution-dispatch-v1";
const MAX_CANDIDATES: i64 = 256;

/// One authenticated HTTP/MCP call. Clones share its snapshot and decision;
/// neither the context nor its operation identity can be deserialized.
pub(crate) struct RequestContext {
    pub(crate) actor: String,
    pub(crate) principal: Principal,
    pub(crate) operation_id: String,
    drain: OnceLock<Drain>,
    authenticated_key: OnceLock<String>,
    source_credential: OnceLock<String>,
    snapshot: OnceCell<Vec<ApprovalGrant>>,
    authority: OnceCell<Arc<authority::Snapshot>>,
    policies: tokio::sync::Mutex<Vec<(String, PolicyDecision)>>,
    decision: OnceLock<Resolution>,
}

impl std::fmt::Debug for RequestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RequestContext { [REDACTED] }")
    }
}

pub(crate) enum Resolution {
    Gap,
    Covered(Arc<Permit>),
}

struct Drain {
    db: Database,
    chat: Option<acks::ChatAuthority>,
}

impl RequestContext {
    pub(crate) fn participating(&self) -> bool {
        self.drain.get().is_some()
    }

    /// Even a legacy-approved gap cannot downgrade marked in-flight work after
    /// enrollment is disabled. Unsupported and unmarked paths never get here.
    pub(crate) fn check_drain<'a>(&'a self) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if self.permit().is_some() {
                return Ok(());
            } // Covered has stronger transactional fences.
            let Some(drain) = self.drain.get() else {
                return Ok(());
            };
            require_enrolled(&drain.db, &self.actor, None).await?;
            super::security_readiness_service::require(&drain.db, READINESS).await?;
            super::security_readiness_service::require(
                &drain.db,
                super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
            )
            .await?;
            if let Some(chat) = &drain.chat {
                chat.ensure_live_turn(&drain.db, None, None, "execution_permission.drain")
                    .await?;
                if let Some(task) = &chat.execution_task {
                    tasks::check(&drain.db, task).await?;
                }
            }
            Ok(())
        })
    }
    pub(crate) fn enrolled(
        marker: Option<u32>,
        actor: &str,
        key: &str,
        chat: Option<&acks::ChatAuthority>,
    ) -> Option<Arc<Self>> {
        if marker != Some(VERSION)
            || chat.is_some_and(|c| {
                c.execution_unsupported
                    || c.guest
                    || c.platform_thread
                    || c.channel_adapter_origin.is_some()
                    || c.confirmation_policy.is_some()
            })
        {
            return None;
        }
        Some(Arc::new(Self {
            actor: actor.into(),
            principal: chat.map_or_else(
                || Principal::ApiKey(key.into()),
                |c| Principal::Agent(c.agent_id.clone()),
            ),
            operation_id: Uuid::new_v4().to_string(),
            drain: OnceLock::new(),
            authenticated_key: OnceLock::new(),
            source_credential: OnceLock::new(),
            snapshot: OnceCell::new(),
            authority: OnceCell::new(),
            policies: tokio::sync::Mutex::new(Vec::new()),
            decision: OnceLock::new(),
        }))
    }

    pub(crate) fn bind_key(&self, key: &crate::models::api_key::ApiKey, credential: Option<&str>) {
        let _ = self.authenticated_key.set(authority::key_digest(key));
        if let Some(credential) = credential {
            let _ = self.source_credential.set(credential.to_owned());
        }
    }

    pub(crate) fn permit(&self) -> Option<&Arc<Permit>> {
        match self.decision.get() {
            Some(Resolution::Covered(permit)) => Some(permit),
            _ => None,
        }
    }

    async fn authority(
        &self,
        db: &Database,
        connection: &str,
        key: &str,
    ) -> AppResult<&Arc<authority::Snapshot>> {
        self.authority
            .get_or_try_init(|| async {
                let snapshot = authority::Snapshot::load(
                    db,
                    connection,
                    key,
                    self.source_credential.get().map(String::as_str),
                )
                .await?;
                if let Some(expected) = self.authenticated_key.get() {
                    snapshot.require_key(expected)?;
                }
                Ok(Arc::new(snapshot))
            })
            .await
    }

    async fn candidates(&self, db: &Database, connection: &str) -> AppResult<&Vec<ApprovalGrant>> {
        self.snapshot.get_or_try_init(|| async {
            require_enrolled(db, &self.actor, None).await?;
            super::security_readiness_service::require(db, READINESS).await?;
            super::security_readiness_service::require(db, super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY).await?;
            let rows: Vec<ApprovalGrant> = db.collection::<ApprovalGrant>("approval_grants")
                .find(doc! {"execution.version":{"$exists":true},"execution.v1.subject.actor_user_id": &self.actor, "execution.v1.binding.connection_id":connection,
                    "execution.v1.subject.principal": bson::to_bson(&self.principal).map_err(encoding)?})
                .limit(MAX_CANDIDATES + 1).await?.try_collect().await?;
            if rows.len() > MAX_CANDIDATES as usize {
                return Err(AppError::Internal("Execution permission candidate limit exceeded".into()));
            }
            Ok(rows)
        }).await
    }
}

fn encoding(_: impl std::fmt::Display) -> AppError {
    AppError::Internal("Execution permission encoding failed".into())
}
fn refused() -> AppError {
    AppError::Forbidden("Execution permission is no longer valid for this operation".into())
}
fn uncertain() -> AppError {
    AppError::Conflict("Execution outcome is uncertain; the operation will not be repeated".into())
}

/// Group delivery never inherits a private audience merely because its turn
/// predates task enrollment. Legacy groups have no membership revision, so bind
/// their exact roster instead of assigning a permanent revision of one.
pub(crate) fn group_audience(group: &crate::models::assistant_group::AssistantGroup) -> Audience {
    let revision = group.collaboration.as_ref().map_or_else(
        || {
            use sha2::{Digest, Sha256};
            let mut people = group.participant_user_ids.clone();
            people.push(group.user_id.clone());
            people.sort();
            people.dedup();
            let mut agents = group.member_agent_ids.clone();
            agents.sort();
            let digest = Sha256::digest(
                serde_json::to_vec(&(people, agents)).expect("roster serialization"),
            );
            (i64::from_be_bytes(digest[..8].try_into().expect("digest prefix")) & i64::MAX).max(1)
        },
        |c| c.membership_revision,
    );
    Audience::Group {
        group_id: group.id.clone(),
        revision,
    }
}

async fn audience(
    db: &Database,
    chat: Option<&acks::ChatAuthority>,
    actor: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<Audience> {
    let Some(id) = chat.and_then(|c| c.execution_group_id.as_deref()) else {
        return Ok(Audience::Private {
            user_id: actor.into(),
        });
    };
    let groups =
        db.collection::<crate::models::assistant_group::AssistantGroup>("assistant_groups");
    let filter = doc! {"_id":id,"$or":[{"user_id":actor},{"participant_user_ids":actor}]};
    let group = if let Some(session) = session {
        groups
            .find_one_and_update(filter, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
            .session(session)
            .await?
    } else {
        groups.find_one(filter).await?
    }
    .ok_or_else(refused)?;
    Ok(group_audience(&group))
}

async fn require_enrolled(
    db: &Database,
    actor: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let filter = doc! {"_id":actor,"is_active":true,"user_type":"person","execution_permission_enrollment":1_i32};
    // A write serializes disabling enrollment against a provider dispatch barrier.
    let matched = if let Some(session) = session {
        db.collection::<Document>("users")
            .update_one(filter, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
            .session(session)
            .await?
            .matched_count
            == 1
    } else {
        db.collection::<Document>("users")
            .find_one(filter)
            .projection(doc! {"_id":1})
            .await?
            .is_some()
    };
    if !matched {
        return Err(refused());
    }
    Ok(())
}

/// Prepared, concrete service request. Resource verification and unsupported
/// runtime adapters never enter this path by relabelling an operation as HTTP.
pub(crate) struct Effect<'a> {
    pub connection: &'a str,
    pub owner: &'a str,
    pub target: &'a proxy_service::ProxyTarget,
    pub method: &'a str,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub body: &'a [u8],
    pub content_type: Option<&'a str>,
    pub headers: &'a [(String, String)],
    pub endpoint: Option<&'a str>,
    pub floors: &'a [ExecutionFloor],
}

/// Private coverage receipt; only this module constructs one. Actual dispatch
/// rechecks the live permission and binding and serializes one-use consumption.
pub(crate) struct Permit {
    db: Database,
    keys: Arc<EncryptionKeys>,
    grant: ApprovalGrant,
    subject: Subject,
    binding: ConnectionBinding,
    audience: Audience,
    conversation: String,
    operation_id: String,
    digest: String,
    target_digest: String,
    method: String,
    path: String,
    query: Option<String>,
    body: Vec<u8>,
    content_type: Option<String>,
    endpoint: Option<String>,
    catalog: Option<String>,
    floors: Vec<ExecutionFloor>,
    headers: Vec<(String, String)>,
    chat: Option<acks::ChatAuthority>,
    key: String,
    source_credential: Option<String>,
    claim_id: String,
    authority: Arc<authority::Snapshot>,
}

impl Permit {
    pub(crate) fn satisfies(&self, floor: ExecutionFloor, connection: &str) -> bool {
        self.binding.connection_id == connection && self.floors.contains(&floor)
    }
}

pub(crate) fn covers(chat: &acks::ChatAuthority, floor: ExecutionFloor, connection: &str) -> bool {
    chat.execution_permission
        .as_ref()
        .and_then(|c| c.permit())
        .is_some_and(|p| p.satisfies(floor, connection))
}

pub(crate) fn risk_gate(
    chat: &acks::ChatAuthority,
    connection: &str,
    gate: acks::Gate,
) -> acks::Gate {
    if !covers(chat, ExecutionFloor::AccessModeRisk, connection) {
        return gate;
    }
    match gate {
        acks::Gate::Ask(mut need) => {
            need.mode = None;
            if need.webhook
                || need.policy
                || need.delegation
                || need.org_instruction == acks::OrgInstructionRequirement::Required
            {
                acks::Gate::Ask(need)
            } else {
                acks::Gate::Proceed
            }
        }
        other => other,
    }
}

fn prepared_headers(headers: &axum::http::HeaderMap) -> AppResult<Vec<(String, String)>> {
    let mut values: Vec<_> = headers
        .iter()
        .map(|(name, value)| {
            Ok((
                name.as_str().to_owned(),
                value.to_str().map_err(|_| refused())?.to_owned(),
            ))
        })
        .collect::<AppResult<_>>()?;
    values.sort();
    Ok(values)
}

fn target_digest(target: &proxy_service::ProxyTarget) -> String {
    // Exclude refreshable credential bytes; the credential ID/epoch is bound separately.
    super::mcp_service::canonical_sha256(json!({
        "catalog":target.service.id,"base":target.base_url,"destination":target.target_id,
        "auth":target.auth_method,"key_name":target.auth_key_name,
        "catalog_headers":target.catalog_default_headers,"connection_headers":target.user_service_default_headers,
        "policy":target.service.proxy_operation_policy,
        "forward_access_token":target.service.forward_access_token,
        "delegation":target.service.inject_delegation_token,
        "identity":target.service.identity_propagation_mode,
    }))
}

async fn binding(
    db: &Database,
    keys: &EncryptionKeys,
    context: &RequestContext,
    chat: Option<&acks::ChatAuthority>,
    key: &str,
    connection: &str,
    owner: &str,
) -> AppResult<ConnectionBinding> {
    let live_key = db
        .collection::<crate::models::api_key::ApiKey>("api_keys")
        .find_one(doc! {"_id":key,"user_id":&context.actor,"is_active":true})
        .await?
        .ok_or_else(refused)?;
    if live_key.expires_at.is_some_and(|end| end <= Utc::now()) {
        return Err(refused());
    }
    let live_access = super::org_agent_service::resolve_credential_access(db, &live_key).await?;
    let resolved = proxy_service::read_proxy_authority_snapshot_with_access(
        db,
        keys,
        &context.actor,
        connection,
        None,
        live_access.as_deref(),
    )
    .await?
    .ok_or_else(refused)?;
    let nodes = super::node_routing_service::list_configured_binding_node_ids(
        db,
        owner,
        resolved.target.service.id.as_str(),
    )
    .await?;
    let override_identity = if chat.is_none_or(|c| c.org_agent_access.is_none()) {
        super::proxy_service::read_agent_credential_override_identity(
            db,
            &context.actor,
            key,
            connection,
            &resolved.target,
        )
        .await?
        .map(|v| super::execution_authority::OverrideCredentialIdentity {
            api_key_id: v.api_key_id,
            credential_epoch: v.credential_epoch,
        })
    } else {
        None
    };
    let projection =
        super::execution_authority::build_projection(&resolved, override_identity.as_ref(), nodes);
    Ok(contract::connection_binding(
        owner,
        &projection,
        live_access
            .as_ref()
            .and_then(|a| a.shared())
            .map(super::shared_agent_consent::behavior),
    ))
}

pub(crate) fn prepare<'a>(
    db: &'a Database,
    keys: &'a Arc<EncryptionKeys>,
    context: Option<&'a Arc<RequestContext>>,
    chat: Option<&'a acks::ChatAuthority>,
    key: &'a str,
    effect: Effect<'a>,
) -> futures::future::BoxFuture<'a, AppResult<Option<Arc<Permit>>>> {
    Box::pin(async move {
        let Some(context) = context else {
            return Ok(None);
        };
        if let Some(result) = context.decision.get() {
            return match result {
                Resolution::Gap => Ok(None),
                Resolution::Covered(permit)
                    if permit.binding.connection_id == effect.connection
                        && permit.method == effect.method
                        && permit.path == effect.path
                        && permit.query.as_deref() == effect.query
                        && permit.body == effect.body =>
                {
                    Ok(Some(permit.clone()))
                }
                _ => Err(refused()),
            };
        }
        let rows = context.candidates(db, effect.connection).await?;
        if let Some(task) = chat.and_then(|c| c.execution_task.as_ref()) {
            tasks::check(db, task).await?;
        }
        if chat.is_some_and(|c| c.collaboration_thread && c.execution_task.is_none()) {
            return Err(refused());
        }
        let authority = context.authority(db, effect.connection, key).await?.clone();
        let binding = binding(
            db,
            keys,
            context,
            chat,
            key,
            effect.connection,
            effect.owner,
        )
        .await?;
        if !authority.same(
            &authority::Snapshot::load(
                db,
                effect.connection,
                key,
                context.source_credential.get().map(String::as_str),
            )
            .await?,
        ) {
            return Err(refused());
        }
        let subject = Subject {
            actor_user_id: context.actor.clone(),
            principal: context.principal.clone(),
            lane: if chat.is_some_and(|c| {
                c.org_agent_access
                    .as_ref()
                    .is_some_and(|a| a.shared().is_some())
            }) {
                ExecutionLane::Shared
            } else if chat.is_some_and(|c| c.org_agent_access.is_some()) {
                ExecutionLane::OrgMember
            } else {
                ExecutionLane::Owner
            },
        };
        let audience = audience(db, chat, &context.actor, None).await?;
        if chat
            .and_then(|c| c.execution_task.as_ref())
            .is_some_and(|t| t.audience != audience)
        {
            return Err(refused());
        }
        let conversation = chat.map_or("", |c| c.conversation_id.as_str());
        // MCP generic paths have already passed their literal-path check;
        // declared endpoints may contain one builder-owned encoding layer.
        let path = if effect.endpoint.is_some() {
            CanonicalPath::from_mcp_built(effect.path)?
        } else {
            CanonicalPath::from_rest_decoded(effect.path)?
        };
        let digest = super::mcp_service::canonical_sha256(
            json!({"method":effect.method,"path":effect.path,
        "query":effect.query,"body":effect.body,"headers":effect.headers,"endpoint":effect.endpoint,
        "target":target_digest(effect.target),"subject":subject,"binding":binding,"audience":audience}),
        );
        let mut candidates: Vec<_> = rows.iter().collect();
        candidates.sort_by_key(|row| {
            match row
                .execution
                .as_ref()
                .and_then(|e| e.v1.as_ref())
                .map(|c| &c.validity)
            {
                Some(Validity::Persistent) => 0,
                Some(Validity::Window { .. }) => 1,
                Some(Validity::Task { .. }) => 2,
                _ => 3,
            }
        });
        let mut prohibited = None;
        let mut gap = rows.is_empty();
        let mut failure = None;
        for grant in candidates {
            let claim = match grant.execution_use.as_ref() {
                None => contract::ClaimState::Available,
                Some(use_)
                    if use_.version != VERSION
                        || matches!(
                            use_.state,
                            ExecutionUseState::Dispatched | ExecutionUseState::Unknown
                        ) =>
                {
                    contract::ClaimState::Uncertain
                }
                _ => contract::ClaimState::Used,
            };
            let request = contract::Request {
                now: Utc::now(),
                subject: &subject,
                binding: &binding,
                audience: &audience,
                conversation_id: conversation,
                operation: contract::PreparedOperation {
                    id: &context.operation_id,
                    digest: &digest,
                    catalog_service_id: Some(&effect.target.service.id),
                    endpoint_id: effect.endpoint,
                    method: effect.method,
                    path: &path,
                    query: effect.query,
                    body: effect.body,
                    content_type: effect.content_type,
                },
                task: chat.and_then(|c| c.execution_task.as_ref()).map(|t| {
                    contract::TaskEvidence {
                        task: &t.task,
                        actor_user_id: &t.actor_user_id,
                        audience: &t.audience,
                        hard_deadline: t.hard_deadline,
                        active: !t.cancelled,
                        admitted_descendant: true,
                    }
                }),
                resource: None,
                floors: effect.floors,
                authority: contract::LiveAuthority::Allowed,
                eligible_decider: true,
                claim,
                delegation_ceilings: &[],
            };
            match contract::evaluate(grant, &request) {
                Ok(contract::Decision::Covered { .. }) => {
                    let mut held_chat = chat.cloned();
                    // Break the context -> permit -> chat -> context reference cycle.
                    if let Some(chat) = held_chat.as_mut() {
                        chat.execution_permission = None;
                    }
                    let permit = Arc::new(Permit {
                        db: db.clone(),
                        keys: keys.clone(),
                        grant: grant.clone(),
                        subject,
                        binding,
                        audience,
                        conversation: conversation.into(),
                        operation_id: context.operation_id.clone(),
                        digest,
                        target_digest: target_digest(effect.target),
                        method: effect.method.into(),
                        path: effect.path.into(),
                        query: effect.query.map(str::to_owned),
                        body: effect.body.to_vec(),
                        content_type: effect.content_type.map(str::to_owned),
                        endpoint: effect.endpoint.map(str::to_owned),
                        catalog: Some(effect.target.service.id.clone()),
                        floors: effect.floors.to_vec(),
                        headers: effect.headers.to_vec(),
                        chat: held_chat,
                        key: key.into(),
                        source_credential: context.source_credential.get().cloned(),
                        claim_id: Uuid::new_v4().to_string(),
                        authority,
                    });
                    context
                        .decision
                        .set(Resolution::Covered(permit.clone()))
                        .map_err(|_| refused())?;
                    return Ok(Some(permit));
                }
                Ok(contract::Decision::RequestableGap(_)) => gap = true,
                Ok(contract::Decision::Prohibited(reason)) => {
                    prohibited.get_or_insert(reason);
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if let Some(error) = failure {
            return Err(match error {
                contract::EvaluationError::OutcomeUncertain => uncertain(),
                contract::EvaluationError::ResourceVerificationRequired => AppError::Forbidden(
                    "Execution permission resource adapter is unsupported in this phase".into(),
                ),
                contract::EvaluationError::AuthorityUnavailable => {
                    AppError::Internal("Execution authority unavailable".into())
                }
            });
        }
        if !gap && prohibited.is_some() {
            return Err(refused());
        }
        let _ = context.decision.set(Resolution::Gap);
        Ok(None)
    })
}

impl Permit {
    /// No side effects: refresh the binding after waits/credential resolution.
    pub(crate) fn revalidate<'a>(
        &'a self,
        target: &'a proxy_service::ProxyTarget,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if target_digest(target) != self.target_digest {
                return Err(refused());
            }
            require_enrolled(&self.db, &self.subject.actor_user_id, None).await?;
            super::security_readiness_service::require(&self.db, READINESS).await?;
            super::security_readiness_service::require(
                &self.db,
                super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
            )
            .await?;
            if let Some(chat) = &self.chat {
                chat.ensure_live_turn(&self.db, None, None, "execution_permission.egress")
                    .await?;
                if let Some(task) = &chat.execution_task {
                    tasks::check(&self.db, task).await?;
                }
            }
            if audience(
                &self.db,
                self.chat.as_ref(),
                &self.subject.actor_user_id,
                None,
            )
            .await?
                != self.audience
            {
                return Err(refused());
            }
            let context = RequestContext {
                actor: self.subject.actor_user_id.clone(),
                principal: self.subject.principal.clone(),
                operation_id: self.operation_id.clone(),
                drain: OnceLock::new(),
                authenticated_key: OnceLock::new(),
                source_credential: OnceLock::new(),
                snapshot: OnceCell::new(),
                authority: OnceCell::new(),
                policies: tokio::sync::Mutex::new(Vec::new()),
                decision: OnceLock::new(),
            };
            let current = binding(
                &self.db,
                &self.keys,
                &context,
                self.chat.as_ref(),
                &self.key,
                &self.binding.connection_id,
                &self.binding.owner_user_id,
            )
            .await?;
            if current != self.binding {
                return Err(refused());
            }
            let grant = self
                .db
                .collection::<ApprovalGrant>("approval_grants")
                .find_one(doc! {"_id":&self.grant.id})
                .await?
                .ok_or_else(refused)?;
            self.validate_grant(&grant)?;
            Ok(())
        })
    }

    fn validate_grant(&self, grant: &ApprovalGrant) -> AppResult<()> {
        if let Some(use_) = &grant.execution_use {
            if use_.version != VERSION
                || matches!(
                    use_.state,
                    ExecutionUseState::Dispatched | ExecutionUseState::Unknown
                )
            {
                return Err(uncertain());
            }
            if use_.claim_id != self.claim_id || use_.state != ExecutionUseState::Claimed {
                return Err(AppError::Conflict(
                    "Execution permission has already been claimed".into(),
                ));
            }
        }
        if grant
            .execution
            .as_ref()
            .and_then(|e| e.v1.as_ref())
            .map(|c| c.revision)
            != self
                .grant
                .execution
                .as_ref()
                .and_then(|e| e.v1.as_ref())
                .map(|c| c.revision)
        {
            return Err(refused());
        }
        let path = if self.endpoint.is_some() {
            CanonicalPath::from_mcp_built(&self.path)?
        } else {
            CanonicalPath::from_rest_decoded(&self.path)?
        };
        let claim = match grant.execution_use.as_ref() {
            None => contract::ClaimState::Available,
            Some(use_)
                if use_.version == VERSION
                    && use_.claim_id == self.claim_id
                    && use_.state == ExecutionUseState::Claimed =>
            {
                contract::ClaimState::Available
            }
            Some(use_)
                if matches!(
                    use_.state,
                    ExecutionUseState::Dispatched | ExecutionUseState::Unknown
                ) =>
            {
                contract::ClaimState::Uncertain
            }
            _ => contract::ClaimState::Used,
        };
        let request = contract::Request {
            now: Utc::now(),
            subject: &self.subject,
            binding: &self.binding,
            audience: &self.audience,
            conversation_id: &self.conversation,
            operation: contract::PreparedOperation {
                id: &self.operation_id,
                digest: &self.digest,
                catalog_service_id: self.catalog.as_deref(),
                endpoint_id: self.endpoint.as_deref(),
                method: &self.method,
                path: &path,
                query: self.query.as_deref(),
                body: &self.body,
                content_type: self.content_type.as_deref(),
            },
            task: self
                .chat
                .as_ref()
                .and_then(|c| c.execution_task.as_ref())
                .map(|t| contract::TaskEvidence {
                    task: &t.task,
                    actor_user_id: &t.actor_user_id,
                    audience: &t.audience,
                    hard_deadline: t.hard_deadline,
                    active: !t.cancelled,
                    admitted_descendant: true,
                }),
            resource: None,
            floors: &self.floors,
            authority: contract::LiveAuthority::Allowed,
            eligible_decider: false,
            claim,
            delegation_ceilings: &[],
        };
        match contract::evaluate(grant, &request) {
            Ok(contract::Decision::Covered { .. }) => Ok(()),
            Err(contract::EvaluationError::OutcomeUncertain) => Err(uncertain()),
            _ => Err(refused()),
        }
    }

    /// Serialize revocation, turn cancellation and the one-use dispatch marker.
    /// This is called at the actual provider-send boundary, never in a prompt.
    pub(crate) fn before_send<'a>(
        &'a self,
        target: &'a proxy_service::ProxyTarget,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            self.revalidate(target).await?;
            self.reserve().await?;
            let mut session = self.db.client().start_session().await?;
            session
                .start_transaction()
                .and_run(self, |session, permit| {
                    Box::pin(async move {
                        super::api_key_mutation_service::transaction_result(
                            permit.claim_in_session(session, true).await,
                        )
                    })
                })
                .await
                .map_err(super::api_key_mutation_service::map_transaction_error)?;
            self.after_barrier().await
        })
    }

    /// Mongo snapshots do not predicate-lock absent rows. Check the committed
    /// selector set after our write barrier too: a newly inserted deny/config
    /// that committed during the transaction must stop this unsent attempt.
    async fn after_barrier(&self) -> AppResult<()> {
        let live = authority::Snapshot::load(
            &self.db,
            &self.binding.connection_id,
            &self.key,
            self.source_credential.as_deref(),
        )
        .await;
        match live {
            Ok(current) if self.authority.same(&current) => Ok(()),
            result => {
                self.finish(ExecutionUseState::NotSent).await?;
                Err(result.err().unwrap_or_else(refused))
            }
        }
    }

    fn reserve<'a>(&'a self) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if !matches!(
                self.grant
                    .execution
                    .as_ref()
                    .and_then(|e| e.v1.as_ref())
                    .map(|c| &c.validity),
                Some(Validity::Operation { .. })
            ) {
                return Ok(());
            }
            let mut session = self.db.client().start_session().await?;
            session
                .start_transaction()
                .and_run(self, |session, permit| {
                    Box::pin(async move {
                        super::api_key_mutation_service::transaction_result(
                            permit.claim_in_session(session, false).await,
                        )
                    })
                })
                .await
                .map_err(super::api_key_mutation_service::map_transaction_error)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_effect(
        &self,
        target: &proxy_service::ProxyTarget,
        method: &str,
        path: &str,
        query: Option<&str>,
        body: &[u8],
        headers: &axum::http::HeaderMap,
    ) -> AppResult<()> {
        let actual = proxy_service::collect_forward_headers_with_defaults(
            headers,
            &[
                target.catalog_default_headers.as_slice(),
                target.user_service_default_headers.as_slice(),
            ],
        );
        if target_digest(target) != self.target_digest
            || method != self.method
            || path != self.path
            || query != self.query.as_deref()
            || body != self.body
            || (self.endpoint.is_none() && actual != self.headers)
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Specialized HTTP transports construct their own request, bypassing the
    /// ordinary proxy sender. Bind the actual URL and body before their barrier.
    pub(crate) fn validate_specialized_request(
        &self,
        target: &proxy_service::ProxyTarget,
        request: &reqwest::Request,
    ) -> AppResult<()> {
        let mut expected = url::Url::parse(&format!(
            "{}/{}",
            target.base_url.trim_end_matches('/'),
            self.path.trim_start_matches('/')
        ))
        .map_err(encoding)?;
        expected.set_query(self.query.as_deref().filter(|q| !q.is_empty()));
        if request.url() != &expected {
            return Err(refused());
        }
        self.validate_effect(
            target,
            request.method().as_str(),
            &self.path,
            self.query.as_deref(),
            request
                .body()
                .and_then(|b| b.as_bytes())
                .unwrap_or_default(),
            request.headers(),
        )
    }

    /// MCP's declared header parameters are merged into its server envelope,
    /// not the ordinary proxy header allowlist. Check their exact preparation
    /// before that merge; the common send path still checks target/body/path.
    pub(crate) fn validate_prepared_headers(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> AppResult<()> {
        let actual = prepared_headers(headers)?;
        if actual != self.headers {
            return Err(refused());
        }
        Ok(())
    }

    fn claim_in_session<'a>(
        &'a self,
        session: &'a mut ClientSession,
        dispatch: bool,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            require_enrolled(&self.db, &self.subject.actor_user_id, Some(session)).await?;
            if audience(
                &self.db,
                self.chat.as_ref(),
                &self.subject.actor_user_id,
                Some(session),
            )
            .await?
                != self.audience
            {
                return Err(refused());
            }
            self.authority.fence(&self.db, session).await?;
            if let Some(chat) = &self.chat {
                let (row, _, _) = Box::pin(acks::fence(&self.db, chat, session)).await?;
                super::assistant_access_policy::require_same_live_turn(chat, &row)?;
                super::assistant_group_service::fence_member_thread(&self.db, &row, session)
                    .await?;
                if let Some(shared) = chat.org_agent_access.as_ref().and_then(|a| a.shared()) {
                    super::shared_agent_service::fence(&self.db, shared, session).await?;
                    let agents = self
                        .db
                        .collection::<crate::models::assistant_agent::AssistantAgent>(
                            "assistant_agents",
                        );
                    let current = agents
                    .find_one(
                        doc! {"_id":&chat.agent_id,"user_id":shared.owner(),"destroyed_at":null},
                    )
                    .session(&mut *session)
                    .await?
                    .ok_or_else(refused)?;
                    if super::shared_agent_consent::behavior_for_agent(&current, shared.revision())
                        != super::shared_agent_consent::behavior(shared)
                    {
                        return Err(refused());
                    }
                    if agents
                        .update_one(
                            doc! {"_id":&current.id},
                            doc! {"$inc":{"execution_dispatch_fence":1_i64}},
                        )
                        .session(&mut *session)
                        .await?
                        .matched_count
                        != 1
                    {
                        return Err(refused());
                    }
                }
            }
            let key = self
                .db
                .collection::<Document>("api_keys")
                .update_one(
                    doc! {"_id":&self.key,"user_id":&self.subject.actor_user_id,"is_active":true,
                    "$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
                    doc! {"$inc":{"execution_dispatch_fence":1_i64}},
                )
                .session(&mut *session)
                .await?;
            if key.matched_count != 1 {
                return Err(refused());
            }
            let collection = self.db.collection::<ApprovalGrant>("approval_grants");
            let current = collection
                .find_one(doc! {"_id":&self.grant.id})
                .session(&mut *session)
                .await?
                .ok_or_else(refused)?;
            self.validate_grant(&current)?;
            let contract = current
                .execution
                .as_ref()
                .and_then(|e| e.v1.as_ref())
                .ok_or_else(refused)?;
            let mut filter = doc! {"_id":&current.id,"revoked":false,"execution.version":VERSION as i32,"execution.v1.revision":contract.revision};
            let update = if matches!(contract.validity, Validity::Operation { .. }) {
                let now = Utc::now();
                if dispatch {
                    filter.insert("execution_use.claim_id", &self.claim_id);
                    filter.insert("execution_use.state", "claimed");
                    doc! {"$set":{"execution_use.state":"dispatched","execution_use.dispatched_at":bson::DateTime::from_chrono(now)}}
                } else {
                    filter.insert("execution_use", bson::Bson::Null);
                    doc! {"$set":{"execution_use":bson::to_bson(&ExecutionUse {
                        version:VERSION, operation_id:self.operation_id.clone(), prepared_digest:self.digest.clone(), claim_id:self.claim_id.clone(),
                        state:ExecutionUseState::Claimed, claimed_at:now, dispatched_at:None, completed_at:None,
                    }).map_err(encoding)?}}
                }
            } else {
                doc! {"$inc":{"execution_dispatch_fence":1_i64}}
            };
            if collection
                .update_one(filter, update)
                .session(&mut *session)
                .await?
                .matched_count
                != 1
            {
                return Err(refused());
            }
            if dispatch {
                let key = super::audit_service::audit_chain_hmac_key()
                    .ok_or_else(|| AppError::Internal("Audit chain unavailable".into()))?;
                super::audit_service::log_actor_event_in_session(&self.db,session,key,&super::audit_service::AuditActor {user_id:self.subject.actor_user_id.clone(),api_key_id:Some(self.key.clone()),api_key_name:None,ip_address:None,user_agent:None},
                "execution_permission_dispatched",json!({"grant_id":self.grant.id,"revision":contract.revision,"operation_id":self.operation_id,"lane":self.subject.lane,"one_use":matches!(contract.validity,Validity::Operation{..})})).await?;
            }
            Ok(())
        })
    }

    pub(crate) async fn complete(&self, received: bool) -> AppResult<()> {
        self.finish(if received {
            ExecutionUseState::Completed
        } else {
            ExecutionUseState::Unknown
        })
        .await
    }

    async fn finish(&self, state: ExecutionUseState) -> AppResult<()> {
        if !matches!(
            self.grant
                .execution
                .as_ref()
                .and_then(|e| e.v1.as_ref())
                .map(|c| &c.validity),
            Some(Validity::Operation { .. })
        ) {
            return Ok(());
        }
        let now = Utc::now();
        self.db.collection::<Document>("approval_grants").update_one(
            doc! {"_id":&self.grant.id,"execution_use.claim_id":&self.claim_id,"execution_use.state":"dispatched"},
            doc! {"$set":{"execution_use.state":bson::to_bson(&state).map_err(encoding)?,"execution_use.completed_at":bson::DateTime::from_chrono(now),
                "revoked":true,"expires_at":bson::DateTime::from_chrono(contract::terminal_expiry(now))}},
        ).await?;
        Ok(())
    }
}

/// Policy resolution is read-only. The old dispatcher remains responsible for
/// prompting on a gap; only a fully covering receipt changes its outcome.
#[allow(clippy::too_many_arguments)]
pub(crate) fn admit_http<'a>(
    state: &'a crate::AppState,
    auth: &'a crate::mw::auth::AuthUser,
    connection: Option<&'a str>,
    owner: Option<&'a str>,
    target: &'a mut proxy_service::ProxyTarget,
    method: &'a str,
    path: &'a str,
    query: Option<&'a str>,
    body: &'a [u8],
    headers: &'a axum::http::HeaderMap,
    endpoint: Option<&'a str>,
    descriptor: &'a super::operation_descriptor::OperationDescriptor,
    auto_connected: bool,
    legacy_operation: Option<&'a serde_json::Value>,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let Some(context) = auth.execution_permission.as_ref() else {
            return Ok(());
        };
        let Some(connection) = connection else {
            return Ok(());
        };
        if method == "CONNECT"
            || headers.contains_key(axum::http::header::UPGRADE)
            || auth.api_key_purpose != crate::models::api_key::ApiKeyPurpose::General
            || !matches!(auth.auth_method, crate::mw::auth::AuthMethod::ApiKey)
            || auth.assistant_chat.as_ref().is_some_and(|c| {
                c.execution_unsupported
                    || c.guest
                    || c.platform_thread
                    || c.channel_adapter_origin.is_some()
                    || c.confirmation_policy.is_some()
            })
        {
            return Ok(());
        }
        if matches!(
            target.auth_method.as_str(),
            nyxid_service_adapters::ifttt::AUTH_METHOD
                | nyxid_service_adapters::ifttt_mcp::AUTH_METHOD
        ) || super::telnyx_platform_service::contract(&target.service).is_some()
        {
            return Ok(());
        }
        let mut drain_chat = auth.assistant_chat.as_deref().cloned();
        if let Some(chat) = drain_chat.as_mut() {
            chat.execution_permission = None;
        }
        let _ = context.drain.set(Drain {
            db: state.db.clone(),
            chat: drain_chat,
        });
        target.execution_request = Some(context.clone());
        // Marked in-flight work cannot downgrade to legacy when enrollment drains,
        // even when today's mode happens to require no discretionary prompt.
        context.candidates(&state.db, connection).await?;
        if let Some(chat) = auth.assistant_chat.as_deref()
            && let Some(task) = &chat.execution_task
        {
            if task.actor_user_id != chat.user_id {
                return Err(refused());
            }
            tasks::check(&state.db, task).await?;
        }
        // Capture policy/credential selectors before evaluating any cached policy.
        // prepare compares them again, and the send transaction fences this same
        // evidence. A deny inserted during admission cannot become a fresh baseline.
        context
            .authority(
                &state.db,
                connection,
                auth.api_key_id.as_deref().ok_or_else(refused)?,
            )
            .await?;
        let actor = auth.effective_approval_owner_user_id();
        let owner = owner.unwrap_or(&actor);
        let (outcome, _) = approval(
            Some(context),
            &state.db,
            &actor,
            owner,
            &target.service.id,
            descriptor,
            auth.approval_requester_type(),
            &auth.approval_requester_id(),
            false,
            auto_connected,
            auth.assistant_chat.as_deref(),
        )
        .await?;
        if matches!(outcome, super::approval_service::ApprovalOutcome::Denied) {
            return Err(refused());
        }
        let mut floors = Vec::new();
        if matches!(outcome, super::approval_service::ApprovalOutcome::NeedsApproval(ref p) if !p.resolution.from_org_policy && owner == actor)
        {
            floors.push(ExecutionFloor::PersonalServiceApproval);
        }
        if let Some(chat) = auth.assistant_chat.as_deref() {
            let facts = acks::ActionFacts::of(match descriptor.verb {
                crate::models::service_approval_config::ApprovalVerb::Read => {
                    acks::OwnerActionClass::Read
                }
                crate::models::service_approval_config::ApprovalVerb::Destructive => {
                    acks::OwnerActionClass::Destructive
                }
                _ => acks::OwnerActionClass::Change,
            });
            if acks::confirmation_needed(&state.db, chat, facts, false)
                .await?
                .ask()
                .is_some_and(|n| n.mode.is_some())
            {
                floors.push(ExecutionFloor::AccessModeRisk);
            }
            if owner == actor
                && chat
                    .org_agent_access
                    .as_ref()
                    .is_some_and(|a| a.shared().is_some())
            {
                // The existing no-credential/platform model exemption remains narrow.
                let model = target.service.inference.is_some()
                    && serde_json::from_slice(body).is_ok_and(|v| {
                        super::assistant_service_execution::stateless_inference(path, &v)
                    });
                let connection_row =
                    super::user_service_service::get_user_service(&state.db, owner, connection)
                        .await?;
                let mut raw_operation = super::assistant_service_execution::http_operation(
                    target, method, path, query, body,
                );
                super::assistant_service_execution::bind_headers(
                    &mut raw_operation,
                    &proxy_service::collect_forward_headers_with_defaults(
                        headers,
                        &[
                            target.catalog_default_headers.as_slice(),
                            target.user_service_default_headers.as_slice(),
                        ],
                    ),
                );
                if model && chat.turn_live {
                    raw_operation["kind"] = json!("raw_inference");
                    raw_operation
                        .as_object_mut()
                        .expect("operation")
                        .remove("body");
                }
                let legacy_covered =
                    if let Some(operation) = legacy_operation.or(Some(&raw_operation)) {
                        let binding = super::assistant_service_execution::connection_binding(
                            &state.db,
                            connection,
                            &chat.api_key_id,
                        )
                        .await?;
                        super::shared_agent_consent::always_covers(
                            &state.db,
                            chat,
                            chat.org_agent_access
                                .as_ref()
                                .and_then(|a| a.shared())
                                .expect("shared"),
                            &connection_row,
                            &binding,
                            operation,
                        )
                        .await?
                    } else {
                        false
                    };
                if !(legacy_covered
                    || model && super::platform_key_service::binding(&connection_row) == "platform")
                {
                    floors.push(ExecutionFloor::SharedConnectionConsent);
                }
            }
        }
        if floors.is_empty() {
            return Ok(());
        }
        let semantic_headers = if endpoint.is_some() {
            prepared_headers(headers)?
        } else {
            proxy_service::collect_forward_headers_with_defaults(
                headers,
                &[
                    target.catalog_default_headers.as_slice(),
                    target.user_service_default_headers.as_slice(),
                ],
            )
        };
        let permit = prepare(
            &state.db,
            &state.encryption_keys,
            Some(context),
            auth.assistant_chat.as_deref(),
            auth.api_key_id.as_deref().ok_or_else(refused)?,
            Effect {
                connection,
                owner,
                target,
                method,
                path,
                query,
                body,
                content_type: headers
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok()),
                headers: &semantic_headers,
                endpoint,
                floors: &floors,
            },
        )
        .await?;
        target.execution_permission = permit;
        Ok(())
    })
}

pub(crate) fn approval_outcome(
    context: Option<&Arc<RequestContext>>,
    owner: &str,
    connection: Option<&str>,
    outcome: super::approval_service::ApprovalOutcome,
) -> super::approval_service::ApprovalOutcome {
    if let Some(permit) = context.and_then(|c| c.permit())
        && owner == permit.subject.actor_user_id
        && connection
            .is_some_and(|id| permit.satisfies(ExecutionFloor::PersonalServiceApproval, id))
        && matches!(&outcome, super::approval_service::ApprovalOutcome::NeedsApproval(p) if !p.resolution.from_org_policy)
    {
        super::approval_service::ApprovalOutcome::Allowed { required: false }
    } else {
        outcome
    }
}

pub(crate) fn approval_for_catalog(
    context: Option<&Arc<RequestContext>>,
    owner: &str,
    catalog: &str,
    outcome: super::approval_service::ApprovalOutcome,
) -> super::approval_service::ApprovalOutcome {
    let connection = context
        .and_then(|c| c.permit())
        .filter(|p| p.catalog.as_deref() == Some(catalog))
        .map(|p| p.binding.connection_id.as_str());
    approval_outcome(context, owner, connection, outcome)
}

// Policy snapshots are per authenticated request, never cached across turns.
type PolicyDecision = (
    super::approval_service::ApprovalOutcome,
    Option<acks::AutoAllowed>,
);
#[allow(clippy::too_many_arguments)]
pub(crate) fn approval<'a>(
    context: Option<&'a Arc<RequestContext>>,
    db: &'a Database,
    actor: &'a str,
    owner: &'a str,
    catalog: &'a str,
    descriptor: &'a super::operation_descriptor::OperationDescriptor,
    requester_type: Option<&'a str>,
    requester_id: &'a str,
    bypass: bool,
    auto_connected: bool,
    chat: Option<&'a acks::ChatAuthority>,
) -> futures::future::BoxFuture<'a, AppResult<PolicyDecision>> {
    Box::pin(async move {
        let Some(context) = context.filter(|c| c.drain.get().is_some()) else {
            return super::approval_service::evaluate_for_chat(
                db,
                actor,
                owner,
                catalog,
                descriptor,
                requester_type,
                requester_id,
                bypass,
                auto_connected,
                chat,
            )
            .await;
        };
        let key = format!(
            "{actor}:{owner}:{catalog}:{descriptor:?}:{requester_type:?}:{requester_id}:{bypass}:{auto_connected}"
        );
        let mut cache = context.policies.lock().await;
        if let Some((_, value)) = cache.iter().find(|(k, _)| k == &key) {
            return Ok(value.clone());
        }
        if owner != actor {
            use crate::models::service_approval_config::ServiceApprovalConfig;
            if let Some(config) = db
                .collection::<ServiceApprovalConfig>("service_approval_configs")
                .find_one(doc! {"user_id":owner,"service_id":catalog})
                .await?
                && super::approval_policy::mandatory_review(&config, descriptor)
            {
                return Err(AppError::Forbidden("Mandatory organization operation review cannot be satisfied by an execution permission".into()));
            }
        }
        let decision = super::approval_service::evaluate_for_chat(
            db,
            actor,
            owner,
            catalog,
            descriptor,
            requester_type,
            requester_id,
            bypass,
            auto_connected,
            chat,
        )
        .await?;
        cache.push((key, decision.clone()));
        Ok(decision)
    })
}

/// Phase-B derived callers keep their legacy path and cannot carry coverage.
pub(crate) fn exclude(auth: &mut crate::mw::auth::AuthUser) {
    auth.execution_enrolled = false;
    auth.execution_permission = None;
    if let Some(chat) = auth.assistant_chat.as_mut() {
        let chat = Arc::make_mut(chat);
        chat.execution_unsupported = true;
        chat.execution_permission = None;
    }
}

#[cfg(test)]
pub(crate) mod tests;
