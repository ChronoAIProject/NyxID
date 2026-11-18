//! Negotiated Assistant Machine HTTP only. A node signature is admission evidence,
//! never a substitute for the original job, task, actor or live authority lease.
use super::*;
use crate::models::{
    execution_proposal::{MachineSource, PreparedAdapter},
    machine_job::{MachineJob, MachineJobBinding},
};
use axum::{body::Body, response::Response};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

pub(crate) const READINESS: &str = "permissions:execution-authoring-machine-v1";
pub(crate) const MIN_WAIT_SECONDS: i64 = 15;

pub(crate) struct Admission {
    pub binding: MachineSource,
    pub deadline: chrono::DateTime<Utc>,
    pub job_deadline: chrono::DateTime<Utc>,
    pub cancellation: CancellationToken,
}
impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MachineAdmission { [REDACTED] }")
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn bind(
    db: &Database,
    auth: &crate::mw::auth::AuthUser,
    job: &MachineJob,
    authority: &Document,
    declared_id: &str,
    request: &nyxid_machine::Request,
    controls: &nyxid_machine::gateway::ExecutionApproval,
    headers: &axum::http::HeaderMap,
    deadline: chrono::DateTime<Utc>,
    cancellation: CancellationToken,
) -> AppResult<()> {
    let Some(context) = &auth.execution_permission else {
        return Ok(());
    };
    if job.execution_task.is_none()
        || !matches!(
            job.binding,
            Some(MachineJobBinding::Assistant { version: 1, .. })
        )
    {
        return Ok(());
    }
    let a: nyxid_machine::authority::Authority =
        bson::from_document(authority.clone()).map_err(encoding)?;
    let node = db
        .collection::<crate::models::node::Node>("nodes")
        .find_one(doc! {"_id":&job.node_id,"is_active":true})
        .await?
        .ok_or_else(refused)?;
    let binding = MachineSource {
        node_owner_id: node.user_id,
        agent_owner_id: a.owner_id,
        job_id: job.id.clone(),
        node_id: job.node_id.clone(),
        runtime_id: job.runtime_id.clone(),
        runtime_generation: job.runtime_generation,
        actor_id: job.user_id.clone(),
        key_id: job.api_key_id.clone(),
        agent_id: job.agent_id.clone(),
        conversation_id: job.conversation_id.clone(),
        turn_id: job.issuing_turn_id.clone().ok_or_else(refused)?,
        context_id: a.context_id,
        generation: a.generation,
        revision: a.revision,
        lease_id: a.lease_id,
        declared_id: declared_id.into(),
    };
    let evidence = proposals::fingerprint(&request.parameters)?;
    context
        .machine
        .set(Arc::new(Admission {
            binding: binding.clone(),
            deadline,
            job_deadline: job.expires_at,
            cancellation: cancellation.clone(),
        }))
        .map_err(|_| refused())?;
    context.bind_cancellation(cancellation);
    if controls.mode == nyxid_machine::gateway::ApprovalMode::Async {
        context
            .async_requested
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // Classify invalid dedupe locators before fixing the Machine adapter;
    // retain B1's Unsupported/legacy fallback for non-retainable controls.
    context.capture_idempotency(headers);
    let _ = context.ingress.set(PreparedAdapter::Machine {
        binding: Box::new(binding),
        call_id: request.request_id.clone(),
        verified_digest: evidence,
    });
    context.enable_authoring();
    Ok(())
}

impl MachineSource {
    fn job_filter(&self) -> Document {
        doc! {"_id":&self.job_id,"user_id":&self.actor_id,"api_key_id":&self.key_id,
        "agent_id":&self.agent_id,"conversation_id":&self.conversation_id,"issuing_turn_id":&self.turn_id,
        "node_id":&self.node_id,"runtime_id":&self.runtime_id,"runtime_generation":self.runtime_generation.map(|v|v as i64),
        "binding.kind":"assistant","binding.version":1,"state":"running","expires_at":{"$gt":bson::DateTime::now()},
        "machine_authority.context_id":&self.context_id,"machine_authority.generation":self.generation as i64,
        "machine_authority.revision":self.revision,"machine_authority.lease_id":&self.lease_id,
        "services.id":&self.declared_id}
    }
    fn selectors(&self, task: &TaskAdmission) -> AppResult<Vec<(&'static str, Document)>> {
        Ok(vec![
            (
                "users",
                doc! {"_id":&self.actor_id,"user_type":"person","is_active":true,"platform_personal_deleting":{"$ne":true}},
            ),
            (
                "nodes",
                doc! {"_id":&self.node_id,"user_id":&self.node_owner_id,"is_active":true,"status":"online",
                "machine.version":nyxid_machine::PROTOCOL_VERSION as i32,"machine.shell":true,"machine.runtime_id":&self.runtime_id,
                "connection_owner.capabilities_resolved":true,"connection_owner.machine_execution_approval_v1":true,
                "connection_owner.expires_at":{"$gt":bson::DateTime::now()}},
            ),
            (
                "assistant_agents",
                doc! {"_id":&self.agent_id,"user_id":&self.agent_owner_id,"destroyed_at":null,
                format!("machine_access.assignments.{}.revision",self.node_id):self.revision,
                format!("machine_access.assignments.{}.capabilities.shell",self.node_id):true},
            ),
            (
                crate::models::machine_access::LEASES,
                doc! {"_id":&self.lease_id,"job_id":&self.job_id,"node_id":&self.node_id,"api_key_id":&self.key_id,
                "authority.actor_id":&self.actor_id,"authority.agent_id":&self.agent_id,"authority.runtime_id":&self.runtime_id,
                "authority.context_id":&self.context_id,"authority.generation":self.generation as i64,"authority.revision":self.revision,
                "authority.expires_at_ms":{"$gt":Utc::now().timestamp_millis()}},
            ),
            (
                crate::models::machine_access::CONTEXTS,
                doc! {"_id":&self.context_id,"node_id":&self.node_id,"actor_id":&self.actor_id,"agent_id":&self.agent_id,
                "$or":[{"mode":"shared_legacy"},{"mode":"separated","generation":self.generation as i64}]},
            ),
            (
                "api_keys",
                doc! {"_id":&self.key_id,"user_id":&self.actor_id,"is_active":true,"$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
            ),
            (
                "assistant_conversations",
                doc! {"_id":&self.conversation_id,"user_id":&self.actor_id,"credential_api_key_id":&self.key_id},
            ),
            (tasks::collection(&task.task), tasks::live_filter(task)?),
        ])
    }
    pub(crate) async fn job(&self, db: &Database) -> AppResult<MachineJob> {
        db.collection::<MachineJob>("machine_jobs")
            .find_one(self.job_filter())
            .await?
            .ok_or_else(refused)
    }
    /// One indexed aggregate; each joined selector starts with a unique _id.
    /// No transaction, write, provider call or per-chunk credential resolution.
    pub(crate) async fn live(&self, db: &Database, task: &TaskAdmission) -> AppResult<()> {
        self.live_with_authority(db, task, None).await
    }
    async fn live_with_authority(
        &self,
        db: &Database,
        task: &TaskAdmission,
        authority: Option<&authority::Snapshot>,
    ) -> AppResult<()> {
        let mut job_filter = self.job_filter();
        job_filter.insert("execution_task", bson::to_bson(task).map_err(encoding)?);
        let mut pipeline = vec![doc! {"$match":job_filter}];
        for (i, (collection, selector)) in self.selectors(task)?.into_iter().enumerate() {
            let name = format!("f{i}");
            pipeline.push(doc!{"$lookup":{"from":collection,"pipeline":[{"$match":selector},{"$project":{"_id":1}}],"as":&name}});
            pipeline.push(doc! {"$match":{format!("{name}.0"): {"$exists":true}}});
        }
        // Owner control is a deny-only indexed node lookup; absence is allowed.
        let desktops = [None, Some(self.context_id.as_str())]
            .into_iter()
            .flat_map(|context| {
                [
                    nyxid_machine::desktop::Display::Secure,
                    nyxid_machine::desktop::Display::Dev,
                ]
                .map(|display| {
                    super::super::machine_desktop_service::desktop_id(
                        &self.node_id,
                        context,
                        display,
                    )
                })
            })
            .collect::<Vec<_>>();
        pipeline.push(doc!{"$lookup":{"from":"machine_desktops","pipeline":[{"$match":{"_id":{"$in":desktops},"status":{"$in":["owner","requested","taking","returning"]}}},{"$limit":1},{"$project":{"_id":1}}],"as":"desktop"}});
        pipeline.push(doc! {"$match":{"desktop.0":{"$exists":false}}});
        if let Some(generation) = self.runtime_generation {
            pipeline.push(doc!{"$lookup":{"from":"managed_machines","pipeline":[{"$match":{"_id":&self.node_id,"owner_id":&self.node_owner_id,"runtime_generation":generation as i64}}],"as":"managed"}});
        }
        if let Some(snapshot) = authority {
            pipeline.extend(snapshot.delivery_lookups());
            let orgs = snapshot
                .memberships()?
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            pipeline.push(doc!{"$lookup":{"from":"feature_flag_overrides","pipeline":[{"$match":{"flag_key":super::super::execution_permission_enrollment::FLAG,"definition_id":null,"$or":[{"org_user_id":null,"target_kind":"global"},{"org_user_id":null,"target_kind":"user","target_key":&self.actor_id},{"org_user_id":{"$in":orgs},"$or":[{"target_kind":"org"},{"target_kind":"role","target_key":{"$in":["admin","member","viewer"]}},{"target_kind":"user","target_key":&self.actor_id}]}]}},{"$limit":2049}],"as":"enrollment"}});
        }
        let row = db
            .collection::<Document>("machine_jobs")
            .aggregate(pipeline)
            .max_time(std::time::Duration::from_secs(2))
            .await?
            .try_next()
            .await?
            .ok_or_else(refused)?;
        if self.runtime_generation.is_some() {
            let managed = row
                .get_array("managed")
                .ok()
                .and_then(|rows| rows.first())
                .ok_or_else(refused)?;
            let managed = bson::from_bson(managed.clone()).map_err(encoding)?;
            super::super::remote_machine_runtime::execution_fence(&managed)?;
        }
        if let Some(snapshot) = authority {
            snapshot.validate_delivery(&row)?;
            let rows = row.get_array("enrollment").map_err(|_| refused())?;
            if rows.len() > 2048 {
                return Err(refused());
            }
            let rows=rows.iter().map(|v|bson::from_bson(v.clone()).map_err(encoding)).collect::<AppResult<Vec<crate::models::feature_flag_override::FeatureFlagOverride>>>()?;
            if !super::super::execution_permission_enrollment::resolve(
                &rows,
                &snapshot.memberships()?,
                &self.actor_id,
                true,
            ) {
                return Err(refused());
            }
        }
        Ok(())
    }
    pub(crate) async fn fence(
        &self,
        db: &Database,
        session: &mut ClientSession,
        task: &TaskAdmission,
    ) -> AppResult<()> {
        super::super::security_readiness_service::require_serving(db, READINESS).await?;
        let job = db
            .collection::<MachineJob>("machine_jobs")
            .find_one(self.job_filter())
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        if bson::to_bson(&job.execution_task).map_err(encoding)?
            != bson::to_bson(&Some(task)).map_err(encoding)?
        {
            return Err(refused());
        }
        descendants::Descendant::Job(Box::new(job))
            .check(db, Some(session))
            .await?;
        for (collection, filter) in
            std::iter::once(("machine_jobs", self.job_filter())).chain(self.selectors(task)?)
        {
            if db
                .collection::<Document>(collection)
                .update_one(filter, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
                .session(&mut *session)
                .await?
                .matched_count
                != 1
            {
                return Err(refused());
            }
        }
        let node = db
            .collection::<crate::models::node::Node>("nodes")
            .find_one(doc! {"_id":&self.node_id})
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        super::super::remote_machine_runtime::fence_execution_in_session(db, &node, session)
            .await?;
        super::super::machine_desktop_service::agent_allowed(db, &self.node_id).await?;
        super::super::machine_desktop_service::agent_allowed_for_context(
            db,
            &self.node_id,
            Some(&self.context_id),
        )
        .await?;
        Ok(())
    }
}

pub(crate) async fn ready(state: &crate::AppState, context: &RequestContext) -> AppResult<bool> {
    if context.machine.get().is_none() {
        return Ok(true);
    }
    Ok(
        super::super::security_readiness_service::serving_report(&state.db, READINESS)
            .await?
            .serving_ready,
    )
}

/// Pure cadence: header fence, then one check per second OR per MiB released.
struct Cadence {
    at: std::time::Instant,
    bytes: usize,
}
impl Cadence {
    fn new() -> Self {
        Self {
            at: std::time::Instant::now(),
            bytes: 0,
        }
    }
    fn due(&self) -> bool {
        self.bytes >= 1024 * 1024 || self.at.elapsed() >= std::time::Duration::from_secs(1)
    }
    fn checked(&mut self) {
        self.at = std::time::Instant::now();
        self.bytes = 0;
    }
}

pub(crate) async fn delivery(
    state: &crate::AppState,
    auth: &crate::mw::auth::AuthUser,
    response: Response,
) -> AppResult<Response> {
    let Some(context) = auth.execution_permission.as_ref() else {
        return Ok(response);
    };
    let Some(admission) = context.machine.get() else {
        return Ok(response);
    };
    // Negotiation alone isn't participation: old/fallback responses get zero reads.
    if context.resume().is_none() {
        return Ok(response);
    }
    let task = auth
        .assistant_chat
        .as_ref()
        .and_then(|c| c.execution_task.clone())
        .ok_or_else(refused)?;
    let binding = admission.binding.clone();
    let token = admission.cancellation.clone();
    let authority = context.permit().ok_or_else(refused)?.authority.clone();
    tokio::select! {
        biased;
        _ = token.cancelled() => return Err(AppError::ClientDisconnected),
        result = async {
            require_enrolled(&state.db, &binding.actor_id, None).await?;
            binding.live_with_authority(&state.db, &task, Some(&authority)).await
        } => result?,
    }
    let db = state.db.clone();
    let (parts, body) = response.into_parts();
    let stream: futures::stream::BoxStream<'static, Result<bytes::Bytes, std::io::Error>> =
        Box::pin(async_stream::try_stream! {
            let mut body=body.into_data_stream();let mut cadence=Cadence::new();
            loop {
                let next=tokio::select!{_ = token.cancelled()=>Err(std::io::Error::other("Machine response unavailable")), chunk=body.next()=>Ok(chunk)}?;
                let Some(chunk)=next else{break};
                let mut chunk=chunk.map_err(|_|std::io::Error::other("Machine response unavailable"))?;
                while !chunk.is_empty(){
                    if token.is_cancelled(){Err(std::io::Error::other("Machine response unavailable"))?;}
                    if cadence.due(){
                        tokio::select! {
                            biased;
                            _ = token.cancelled() => Err(std::io::Error::other("Machine response unavailable")),
                            result = binding.live_with_authority(&db,&task,Some(&authority)) => result.map_err(|_|std::io::Error::other("Machine response unavailable")),
                        }?;
                        cadence.checked();
                    }
                    let n=chunk.len().min(1024*1024-cadence.bytes);
                    let part=chunk.split_to(n);cadence.bytes+=n;yield part;
                }
            }
        });
    Ok(Response::from_parts(parts, Body::from_stream(stream)))
}
