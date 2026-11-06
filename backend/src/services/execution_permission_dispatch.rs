//! Opt-in HTTP execution admission. Enrollment is a read-budget hint, never authority.
//! Recoverable assistant gaps use human-authored proposals; other gaps retain legacy prompts.
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
pub(crate) mod descendants;
mod diagnostics;
pub(crate) mod machine;
mod native;
pub(crate) mod proposals;
pub(crate) mod tasks;

pub const READINESS: &str = "permissions:execution-dispatch-v1";
const MAX_CANDIDATES: i64 = 256;

/// One authenticated HTTP/MCP call. Clones share its snapshot and decision;
/// neither the context nor its operation identity can be deserialized.
pub(crate) struct RequestContext {
    pub(crate) actor: String,
    pub(crate) principal: Principal,
    pub(crate) operation_id: String,
    voice_task: bool,
    machine: OnceLock<Arc<machine::Admission>>,
    ingress: OnceLock<crate::models::execution_proposal::PreparedAdapter>,
    idempotency: OnceLock<String>,
    async_requested: std::sync::atomic::AtomicBool,
    cancellation: OnceLock<tokio_util::sync::CancellationToken>,
    connected_resume: OnceLock<Arc<proposals::Resume>>,
    connected_snapshot: OnceCell<Vec<ApprovalGrant>>,
    wait_guard: OnceLock<Arc<proposals::v2::blocking::WaitGuard>>,
    native: Option<Arc<native::Native>>,
    descendant: Option<Arc<descendants::Descendant>>,
    authoring: std::sync::atomic::AtomicBool,
    generated_correlation: OnceLock<String>,
    resume: Option<Arc<proposals::Resume>>,
    drain: OnceLock<Drain>,
    authenticated_key: OnceLock<String>,
    source_credential: OnceLock<String>,
    admission: OnceCell<bool>,
    snapshot: OnceCell<Vec<ApprovalGrant>>,
    authority: OnceCell<Arc<authority::Snapshot>>,
    policies: tokio::sync::Mutex<Vec<(String, PolicyDecision)>>,
    decision: OnceLock<Resolution>,
    observation: diagnostics::Observation,
}

impl std::fmt::Debug for RequestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RequestContext { [REDACTED] }")
    }
}

impl Drop for RequestContext {
    fn drop(&mut self) {
        self.observation.abandoned(self);
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
    /// Set only when the proxy itself generated the header; caller-supplied
    /// correlation IDs remain exact inputs and cannot use this exemption.
    pub(crate) fn bind_generated_correlation(&self, value: &str) {
        let _ = self.generated_correlation.set(value.to_owned());
    }
    pub(crate) fn enable_authoring(&self) {
        if matches!(
            self.descendant.as_deref(),
            Some(descendants::Descendant::Job(_))
        ) && self.machine.get().is_none()
        {
            return;
        }
        self.authoring
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    fn authoring_allowed(&self) -> bool {
        self.authoring.load(std::sync::atomic::Ordering::Relaxed)
            && !matches!(
                self.ingress.get(),
                Some(crate::models::execution_proposal::PreparedAdapter::Unsupported)
            )
    }
    pub(crate) fn for_resume(
        auth: &mut crate::mw::auth::AuthUser,
        resume: proposals::Resume,
    ) -> AppResult<()> {
        if resume.version == 2 {
            return proposals::v2::attach_resume(auth, resume);
        }
        let chat = auth
            .assistant_chat
            .as_deref()
            .filter(|c| proposals::supported_chat(c))
            .ok_or_else(refused)?;
        let context = Self::enrolled(Some(VERSION), &chat.user_id, &chat.api_key_id, Some(chat))
            .ok_or_else(refused)?;
        let mut context = Arc::try_unwrap(context).map_err(|_| refused())?;
        let authenticated = auth.execution_permission.as_ref().ok_or_else(refused)?;
        if let Some(key) = authenticated.authenticated_key.get() {
            let _ = context.authenticated_key.set(key.clone());
        }
        if let Some(credential) = authenticated.source_credential.get() {
            let _ = context.source_credential.set(credential.clone());
        }
        context.operation_id = resume.effect_id.clone();
        if let Some(value) = resume.effect.generated_correlation.as_deref() {
            context.bind_generated_correlation(value);
        }
        context.resume = Some(Arc::new(resume));
        context.enable_authoring();
        let context = Arc::new(context);
        let mut chat = chat.clone();
        chat.execution_permission = Some(context.clone());
        auth.execution_permission = Some(context);
        auth.assistant_chat = Some(Arc::new(chat));
        Ok(())
    }

    /// The hint only opts into this bounded live read. A fresh false result is
    /// legacy; persisted task/proposal/child work must drain instead.
    async fn admit_candidate(
        &self,
        db: &Database,
        chat: Option<&acks::ChatAuthority>,
    ) -> AppResult<bool> {
        self.observation.stage(diagnostics::Stage::Enrollment);
        let enabled = *self
            .admission
            .get_or_try_init(|| async {
                super::execution_permission_enrollment::live_enabled(db, &self.actor, None).await
            })
            .await?;
        if !enabled
            && (self.resume().is_some()
                || self.descendant.is_some()
                || self.participating()
                || chat.is_some_and(|c| c.execution_task.is_some()))
        {
            return Err(refused());
        }
        if !enabled {
            self.observation
                .decide(diagnostics::Decision::new("legacy", "legacy_flag_off"));
        }
        Ok(enabled)
    }

    pub(crate) fn retain_adapter(
        &self,
        adapter: crate::models::execution_proposal::PreparedAdapter,
        headers: &axum::http::HeaderMap,
    ) {
        self.capture_idempotency(headers);
        if self.resume().is_none() {
            let _ = self.ingress.set(adapter);
        }
        self.enable_authoring();
    }
    pub(crate) fn bind_cancellation(&self, token: tokio_util::sync::CancellationToken) {
        let _ = self.cancellation.set(token);
    }
    fn resume(&self) -> Option<&Arc<proposals::Resume>> {
        self.resume.as_ref().or_else(|| self.connected_resume.get())
    }
    pub(crate) fn capture_idempotency(&self, headers: &axum::http::HeaderMap) {
        // Downstream Idempotency-Key and Prefer semantics never opt into
        // background effects. Preserve the ingress choice across prepared MCP
        // headers, which deliberately omit NyxID control headers.
        if headers.get_all("x-nyxid-approval-mode").iter().count() == 1
            && headers
                .get("x-nyxid-approval-mode")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("async"))
        {
            self.async_requested
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if !self
            .async_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        if let Some(value) = headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 128
                    && headers.get_all("idempotency-key").iter().count() == 1
            })
        {
            let _ = self.idempotency.set(value.to_owned());
        } else if headers.contains_key("idempotency-key") && self.resume().is_none() {
            let _ = self
                .ingress
                .set(crate::models::execution_proposal::PreparedAdapter::Unsupported);
            tracing::info!(
                reason = "idempotency_not_retainable",
                "Execution proposal uses legacy prompts"
            );
        }
    }
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
            if let Some(native) = &self.native {
                native.check(&drain.db, None).await?;
            }
            require_enrolled(&drain.db, &self.actor, None).await?;
            super::security_readiness_service::require(&drain.db, READINESS).await?;
            super::security_readiness_service::require(
                &drain.db,
                super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
            )
            .await?;
            if let Some(source) = &self.descendant {
                source.check(&drain.db, None).await?;
            }
            if let Some(chat) = &drain.chat {
                if self.descendant.is_none() {
                    chat.ensure_live_turn(&drain.db, None, None, "execution_permission.drain")
                        .await?;
                }
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
                    || (c.execution_voice_task && c.execution_task.is_none())
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
            ingress: OnceLock::new(),
            idempotency: OnceLock::new(),
            async_requested: std::sync::atomic::AtomicBool::new(false),
            cancellation: OnceLock::new(),
            connected_resume: OnceLock::new(),
            connected_snapshot: OnceCell::new(),
            wait_guard: OnceLock::new(),
            machine: OnceLock::new(),
            voice_task: chat.is_some_and(|c| c.execution_voice_task),
            native: None,
            descendant: None,
            authoring: std::sync::atomic::AtomicBool::new(false),
            generated_correlation: OnceLock::new(),
            resume: None,
            drain: OnceLock::new(),
            authenticated_key: OnceLock::new(),
            source_credential: OnceLock::new(),
            admission: OnceCell::new(),
            snapshot: OnceCell::new(),
            authority: OnceCell::new(),
            policies: tokio::sync::Mutex::new(Vec::new()),
            decision: OnceLock::new(),
            observation: Default::default(),
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
        if let Some(resume) = self.connected_resume.get() {
            return self
                .connected_snapshot
                .get_or_try_init(|| async {
                    require_enrolled(db, &self.actor, None).await?;
                    let row = db
                        .collection::<ApprovalGrant>("approval_grants")
                        .find_one(doc! {"_id": &resume.grant_id, "user_id": &self.actor,
                        "execution.v1.binding.connection_id": connection})
                        .await?
                        .ok_or_else(refused)?;
                    Ok(vec![row])
                })
                .await;
        }
        self.snapshot.get_or_try_init(|| async {
            require_enrolled(db, &self.actor, None).await?;
            super::security_readiness_service::require(db, READINESS).await?;
            super::security_readiness_service::require(db, super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY).await?;
            let rows: Vec<ApprovalGrant> = db.collection::<ApprovalGrant>("approval_grants")
                .find(doc! {"execution.version":{"$exists":true},"execution.v1.subject.actor_user_id": &self.actor, "execution.v1.binding.connection_id":connection,
                    "execution.v1.subject.principal": bson::to_bson(&self.principal).map_err(encoding)?})
                .limit(MAX_CANDIDATES + 1).await?.try_collect().await?;
            if rows.len() > MAX_CANDIDATES as usize {
                self.observation.decide(diagnostics::Decision::new("operational_error", "candidate_limit_exceeded"));
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
    let mut session = session;
    if !super::execution_permission_enrollment::live_enabled(db, actor, session.as_deref_mut())
        .await?
    {
        return Err(refused());
    }
    // Keep the existing account-disable fence, not a flag-change fence. The
    // rollout flag can race dispatch; exact permission/claim fences cannot.
    if let Some(session) = session {
        let filter = doc! {"_id":actor,"is_active":true,"user_type":"person","platform_personal_deleting":{"$ne":true}};
        if db
            .collection::<Document>("users")
            .update_one(filter, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
            .session(session)
            .await?
            .matched_count
            != 1
        {
            return Err(refused());
        }
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
    resume: Option<Arc<proposals::Resume>>,
    generated_correlation: Option<String>,
    native: Option<Arc<native::Native>>,
    descendant: Option<Arc<descendants::Descendant>>,
    machine: Option<Arc<machine::Admission>>,
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
        // This is NyxID control metadata, never an input to the provider.
        // A retained MCP effect can carry it when a separate floor was approved.
        .filter(|(name, _)| name.as_str() != super::assistant_service_execution::ACK_HEADER)
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
    state: &'a crate::AppState,
    context: Option<&'a Arc<RequestContext>>,
    chat: Option<&'a acks::ChatAuthority>,
    key: &'a str,
    effect: Effect<'a>,
) -> futures::future::BoxFuture<'a, AppResult<Option<Arc<Permit>>>> {
    Box::pin(diagnostics::observe(context, async move {
        let Some(context) = context else {
            return Ok(None);
        };
        let db = &state.db;
        let keys = &state.encryption_keys;
        if !context.admit_candidate(db, chat).await? {
            return Ok(None);
        }
        if let Some(native) = &context.native {
            native.require_effect(&effect)?;
        }
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
        context.observation.stage(diagnostics::Stage::Authority);
        let rows = context.candidates(db, effect.connection).await?;
        if let Some(task) = chat.and_then(|c| c.execution_task.as_ref()) {
            context.observation.stage(diagnostics::Stage::Task);
            tasks::check(db, task).await?;
        }
        if chat.is_some_and(|c| c.collaboration_thread && c.execution_task.is_none()) {
            return Err(refused());
        }
        context.observation.stage(diagnostics::Stage::Authority);
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
        let scope_headers = proposals::scope_headers(
            effect.headers,
            context.generated_correlation.get().map(String::as_str),
        );
        let digest = super::mcp_service::canonical_sha256(
            json!({"method":effect.method,"path":effect.path,
        "query":effect.query,"body":effect.body,"headers":scope_headers,"endpoint":effect.endpoint,
        "target":target_digest(effect.target),"subject":subject,"binding":binding,"audience":audience}),
        );
        if context.resume().is_none()
            && context.authoring_allowed()
            && context.descendant.is_none()
            && context.native.is_none()
            && let Some(chat) = chat.filter(|c| proposals::supported_chat(c))
        {
            proposals::existing_effect(
                db,
                chat,
                effect.connection,
                &digest,
                &state.config.frontend_url,
            )
            .await?;
        }
        if context.resume().is_none() && proposals::v2::eligible(context, chat) {
            proposals::v2::existing(state, context, chat, &effect, &digest, &authority).await?;
        }
        // A completed one-operation permission belongs to its occurrence, not
        // to the next status poll or external request. Still evaluate unknown
        // versions and revocations of reusable/current-occurrence permissions.
        let mut candidates: Vec<_> = rows.iter().filter(|grant| {
            !(context.resume().is_none() && proposals::v2::eligible(context, chat)
                && grant.execution.as_ref().is_some_and(|permission| {
                    permission.version == VERSION && permission.v1.as_ref().is_some_and(|contract| {
                        matches!(&contract.validity, Validity::Operation { operation_id, .. } if operation_id != &context.operation_id)
                    })
                }))
        }).collect();
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
        context.observation.stage(diagnostics::Stage::Coverage);
        let mut prohibited = None;
        let mut gap = candidates.is_empty();
        let mut gaps = Vec::new();
        if candidates.is_empty() {
            gaps.push(contract::Gap::MissingPermission);
        }
        let mut failure = None;
        for grant in candidates {
            if let Some(resume) = context.resume() {
                if grant.id != resume.grant_id {
                    continue;
                }
                if digest != resume.prepared_digest || authority.digest() != resume.authority_digest
                {
                    return Err(refused());
                }
            }
            if grant.execution_review.is_some()
                && grant
                    .execution
                    .as_ref()
                    .and_then(|e| e.v1.as_ref())
                    .is_some_and(|c| !matches!(c.validity, Validity::Operation { .. }))
                && !proposals::reusable_shape(
                    effect.method,
                    effect.body,
                    effect.content_type,
                    &scope_headers,
                )
            {
                gap = true;
                gaps.push(contract::Gap::Operation);
                continue;
            }

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
                resource: context
                    .native
                    .as_ref()
                    .map(|n| n.evidence(&digest, &binding)),
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
                        resume: context.resume().cloned(),
                        generated_correlation: context.generated_correlation.get().cloned(),
                        native: context.native.clone(),
                        descendant: context.descendant.clone(),
                        machine: context.machine.get().cloned(),
                    });
                    context
                        .decision
                        .set(Resolution::Covered(permit.clone()))
                        .map_err(|_| refused())?;
                    context
                        .observation
                        .decide(diagnostics::Decision::new("covered", "permission_covered"));
                    return Ok(Some(permit));
                }
                Ok(contract::Decision::RequestableGap(reason)) => {
                    gap = true;
                    if !gaps.contains(&reason) {
                        gaps.push(reason);
                    }
                }
                Ok(contract::Decision::Prohibited(reason)) => {
                    prohibited.get_or_insert(reason);
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if let Some(error) = failure {
            context
                .observation
                .decide(diagnostics::Decision::failure(error));
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
        if !gap && let Some(reason) = prohibited {
            context
                .observation
                .decide(diagnostics::Decision::prohibited(reason));
            return Err(refused());
        }
        if context.resume().is_some() {
            return Err(refused());
        }
        context.observation.decide(diagnostics::Decision::gap(
            gaps.first()
                .copied()
                .unwrap_or(contract::Gap::MissingPermission),
        ));
        context.observation.stage(diagnostics::Stage::Proposal);
        if context.authoring_allowed()
            && context.descendant.is_none()
            && context.native.is_none()
            && let Some(chat) = chat
            && let Some(request) = proposals::create(
                db,
                keys,
                context,
                chat,
                &effect,
                subject.clone(),
                binding.clone(),
                audience.clone(),
                digest.clone(),
                &authority,
                &gaps,
            )
            .await?
        {
            return Err(
                if request.status == "pending" && request.expires_at > Utc::now() {
                    AppError::ApprovalRequired {
                        request_id: request.id,
                    }
                } else {
                    AppError::ApprovalFailed { request_id: request.id, reason: "This operation already has a terminal or saved decision; it will not be repeated".into(), approve_url: proposals::approve_url(&state.config.frontend_url) }
                },
            );
        }
        if let Some(request) = proposals::v2::create(
            state, context, chat, &effect, subject, binding, audience, digest, &authority, &gaps,
        )
        .await?
        {
            if proposals::v2::blocking::wait(state, context, &request).await? {
                return prepare(state, Some(context), chat, key, effect).await;
            }
            return Err(proposals::v2::pending(&request, &state.config.frontend_url));
        }
        let _ = context.decision.set(Resolution::Gap);
        Ok(None)
    }))
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
            if let Some(native) = &self.native {
                native.check(&self.db, None).await?;
            }
            require_enrolled(&self.db, &self.subject.actor_user_id, None).await?;
            super::security_readiness_service::require(&self.db, READINESS).await?;
            super::security_readiness_service::require(
                &self.db,
                super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
            )
            .await?;
            if let Some(source) = &self.descendant {
                source.check(&self.db, None).await?;
            }
            if let Some(chat) = &self.chat {
                if self.descendant.is_none() {
                    chat.ensure_live_turn(&self.db, None, None, "execution_permission.egress")
                        .await?;
                }
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
                ingress: OnceLock::new(),
                idempotency: OnceLock::new(),
                async_requested: std::sync::atomic::AtomicBool::new(false),
                cancellation: OnceLock::new(),
                connected_resume: OnceLock::new(),
                connected_snapshot: OnceCell::new(),
                wait_guard: OnceLock::new(),
                machine: OnceLock::new(),
                voice_task: self.chat.as_ref().is_some_and(|c| c.execution_voice_task),
                native: None,
                descendant: None,
                authoring: std::sync::atomic::AtomicBool::new(false),
                generated_correlation: OnceLock::new(),
                resume: None,
                drain: OnceLock::new(),
                authenticated_key: OnceLock::new(),
                source_credential: OnceLock::new(),
                admission: OnceCell::new(),
                snapshot: OnceCell::new(),
                authority: OnceCell::new(),
                policies: tokio::sync::Mutex::new(Vec::new()),
                decision: OnceLock::new(),
                observation: Default::default(),
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
        if grant.execution_review.is_some()
            && grant
                .execution
                .as_ref()
                .and_then(|e| e.v1.as_ref())
                .is_some_and(|c| !matches!(c.validity, Validity::Operation { .. }))
            && !proposals::reusable_shape(
                &self.method,
                &self.body,
                self.content_type.as_deref(),
                &proposals::scope_headers(&self.headers, self.generated_correlation.as_deref()),
            )
        {
            return Err(refused());
        }
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
            resource: self
                .native
                .as_ref()
                .map(|n| n.evidence(&self.digest, &self.binding)),
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
            self.after_barrier().await?;
            if let Some(guard) = self.resume.as_ref().and_then(|r| r.connected_wait.as_ref()) {
                guard.disarm();
            }
            Ok(())
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
        let result = match live {
            Ok(current) if self.authority.same(&current) => {
                // Only negotiated B2 occurrences add this source check. A job
                // cancellation or owner takeover committed during the barrier
                // must still stop the unsent HTTP effect.
                if let Some(machine) = self.machine.as_ref().filter(|_| self.resume.is_some()) {
                    if machine.cancellation.is_cancelled() {
                        Err(AppError::ClientDisconnected)
                    } else if let Some(task) =
                        self.chat.as_ref().and_then(|c| c.execution_task.as_ref())
                    {
                        tokio::select! {
                            biased;
                            _ = machine.cancellation.cancelled() => Err(AppError::ClientDisconnected),
                            result = machine.binding.live(&self.db, task) => result,
                        }
                    } else {
                        Err(refused())
                    }
                } else {
                    Ok(())
                }
            }
            result => Err(result.err().unwrap_or_else(refused)),
        };
        if result.is_err() {
            self.finish(ExecutionUseState::NotSent).await?;
            if self.machine.is_some()
                && let Some(resume) = &self.resume
            {
                proposals::v2::not_sent_before_handoff(&self.db, resume).await?;
            }
        }
        result
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
            if let Some(native) = &self.native {
                native.check(&self.db, Some(session)).await?;
            }
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
                if let Some(source) = &self.descendant {
                    source.check(&self.db, Some(session)).await?;
                } else {
                    super::assistant_access_policy::require_same_live_turn(chat, &row)?;
                }
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
            if let Some(resume) = &self.resume {
                resume.fence(&self.db, session, dispatch).await?;
            }
            if dispatch {
                self.db
                    .collection::<Document>("approval_grants")
                    .update_one(
                        doc! {"_id":&current.id},
                        doc! {"$set":{"execution_last_used_at":bson::DateTime::now()}},
                    )
                    .session(&mut *session)
                    .await?;
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
        .await?;
        if let Some(resume) = &self.resume
            && resume.connected_wait.is_some()
        {
            proposals::v2::blocking::complete(&self.db, resume, received).await?;
        }
        Ok(())
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
    let observation = auth.execution_permission.as_ref();
    Box::pin(diagnostics::observe(observation, async move {
        let Some(context) = auth.execution_permission.as_ref() else {
            return Ok(());
        };
        let Some(connection) = connection else {
            context.observation.decide(diagnostics::Decision::new(
                "legacy",
                "legacy_unsupported_source",
            ));
            return Ok(());
        };
        if method == "CONNECT"
            || headers.contains_key(axum::http::header::UPGRADE)
            || (auth.api_key_purpose != crate::models::api_key::ApiKeyPurpose::General
                && context.native.is_none())
            || !matches!(auth.auth_method, crate::mw::auth::AuthMethod::ApiKey)
            || auth.assistant_chat.as_ref().is_some_and(|c| {
                c.execution_unsupported
                    || c.guest
                    || c.platform_thread
                    || c.channel_adapter_origin.is_some()
                    || c.confirmation_policy.is_some()
            })
        {
            context.observation.decide(diagnostics::Decision::new(
                "legacy",
                "legacy_unsupported_transport",
            ));
            return Ok(());
        }
        if matches!(
            target.auth_method.as_str(),
            nyxid_service_adapters::ifttt::AUTH_METHOD
                | nyxid_service_adapters::ifttt_mcp::AUTH_METHOD
        ) || super::telnyx_platform_service::contract(&target.service).is_some()
        {
            context.observation.decide(diagnostics::Decision::new(
                "legacy",
                "legacy_unsupported_transport",
            ));
            return Ok(());
        }
        if !context
            .admit_candidate(&state.db, auth.assistant_chat.as_deref())
            .await?
        {
            return Ok(());
        }
        context.observation.stage(diagnostics::Stage::Authority);
        // Preserve the proxy ingress's legacy-card retry decision. This only
        // suppresses new proposal authoring: existing permissions, ceilings and
        // live fences still run, then the legacy gate claims the exact card.
        if !headers.contains_key(super::assistant_service_execution::ACK_HEADER) {
            context.enable_authoring();
        }
        context.capture_idempotency(headers);
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
        context.observation.stage(diagnostics::Stage::Task);
        if let Some(chat) = auth.assistant_chat.as_deref()
            && let Some(task) = &chat.execution_task
        {
            if task.actor_user_id != chat.user_id {
                return Err(refused());
            }
            tasks::check(&state.db, task).await?;
        }
        if let Some(source) = &context.descendant {
            source.check(&state.db, None).await?;
        }
        context.observation.stage(diagnostics::Stage::Authority);
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
        context.observation.stage(diagnostics::Stage::Policy);
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
            context
                .observation
                .decide(diagnostics::Decision::new("hard_denial", "policy_denied"));
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
            if context.descendant.is_none()
                && acks::confirmation_needed(&state.db, chat, facts, false)
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
                        super::shared_agent_consent::has_approved_consent(
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
        if let Some(resume) = context.resume() {
            let grant = context
                .candidates(&state.db, connection)
                .await?
                .iter()
                .find(|g| g.id == resume.grant_id)
                .ok_or_else(refused)?;
            for floor in &grant
                .execution
                .as_ref()
                .and_then(|e| e.v1.as_ref())
                .ok_or_else(refused)?
                .satisfies
            {
                if !floors.contains(floor) {
                    floors.push(*floor);
                }
            }
        }
        if let Some(native) = &context.native {
            native.check(&state.db, None).await?;
        }
        if floors.is_empty()
            && !(context.authoring_allowed()
                && auth
                    .assistant_chat
                    .as_deref()
                    .is_some_and(proposals::supported_chat)
                || proposals::v2::eligible(context, auth.assistant_chat.as_deref()))
        {
            return Ok(());
        }
        let endpoint = context
            .resume()
            .and_then(|r| r.effect.endpoint.as_deref())
            .or(endpoint);
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
            state,
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
    }))
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
        && matches!(&outcome, super::approval_service::ApprovalOutcome::NeedsApproval(p) if !p.resolution.from_org_policy && p.primary_owner_user_id == permit.subject.actor_user_id)
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

#[cfg(test)]
pub(crate) mod phase_b_tests;
