use super::*;

pub(crate) fn view(row: &ApprovalRequest) -> ProposalView {
    let p = proposal(row).ok();
    ProposalView {
        source: p.map(|p| {
            match p.source {
                ProposalSource::Machine { .. } => "Machine service request",
                ProposalSource::External { .. } => "API key request",
                ProposalSource::Native { .. } => "Native permission key",
                ProposalSource::Watch { .. } => "Background operation",
                ProposalSource::Voice { .. } => "Voice task",
            }
            .into()
        }),
        version: 2,
        supported: p.is_some(),
        revision: p.map_or(0, |p| p.revision),
        state: p.map_or_else(
            || "unsupported".into(),
            |p| {
                if row.status == "pending" && row.expires_at <= Utc::now() {
                    "expired".into()
                } else {
                    p.state.clone()
                }
            },
        ),
        can_decide: p.is_some() && row.status == "pending" && row.expires_at > Utc::now(),
        review_path: crate::services::assistant_links::AssistantPage::Approvals.path(),
        review: p.map(|p| p.review.clone()),
        choices: p.map_or_else(Vec::new, |p| {
            p.choices
                .iter()
                .map(|c| ChoiceView {
                    context_scope: None,
                    id: c.id.clone(),
                    digest: c.digest.clone(),
                    label: c.label.clone(),
                    valid_until: (!matches!(c.validity, Validity::Persistent))
                        .then(|| contract::legacy_expiry(&c.validity).to_rfc3339()),
                })
                .collect()
        }),
        grant_id: p.and_then(|p| p.grant_id.clone()),
        failure_code: p.and_then(|p| p.failure_code.clone()),
        result_text: None,
    }
}

#[derive(Serialize)]
pub struct ExecutionView {
    pub version: u32,
    pub state: String,
    pub failure_code: Option<String>,
    pub result_available: bool,
    pub result: Option<ExecutionResult>,
}
impl std::fmt::Debug for ExecutionView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExecutionView { [REDACTED] }")
    }
}

/// Opaque handles locate a receipt; only the original authenticated credential
/// may read it. This function never claims work, enqueues it or sends anything.
pub(crate) async fn readback(
    state: &crate::AppState,
    auth: &crate::mw::auth::AuthUser,
    row: &ApprovalRequest,
) -> AppResult<ExecutionView> {
    let p = proposal(row).map_err(|_| missing())?;
    if auth.auth_method != crate::mw::auth::AuthMethod::ApiKey
        || auth.api_key_id.as_deref() != Some(p.key_id.as_str())
        || auth.api_key_credential_id != p.source_credential_id
        || auth.user_id.to_string() != row.user_id
        || p.delivery_abandoned
    {
        return Err(missing());
    }
    let context = auth.execution_permission.as_ref().ok_or_else(missing)?;
    if context.actor != row.user_id || context.principal != p.contract.subject.principal {
        return Err(missing());
    }
    let snapshot = authority::Snapshot::load(
        &state.db,
        &p.contract.binding.connection_id,
        &p.key_id,
        p.source_credential_id.as_deref(),
    )
    .await
    .map_err(|e| if e.is_forbidden() { missing() } else { e })?;
    if snapshot.digest() != p.authority_digest {
        return Err(missing());
    }
    require_enrolled(&state.db, &row.user_id, None)
        .await
        .map_err(|e| if e.is_forbidden() { missing() } else { e })?;
    if let Some(task) = &p.task {
        tasks::check(&state.db, task)
            .await
            .map_err(|e| if e.is_forbidden() { missing() } else { e })?;
    }
    let binding = match &p.source {
        ProposalSource::Machine { binding } => {
            let admission=context.machine.get().ok_or_else(missing)?;
            if fingerprint(&admission.binding)? != fingerprint(binding)? {return Err(missing());}
            crate::services::security_readiness_service::require(&state.db,machine::READINESS).await?;
            binding.live(&state.db,p.task.as_ref().ok_or_else(missing)?).await.map_err(|e|if e.is_forbidden(){missing()}else{e})?; None
        },
        ProposalSource::External { pool } => pool.as_ref().map(|pool| (
            "service_pools", doc! {"_id":&pool.id,"user_id":&pool.owner_id,"config_revision":pool.revision,"is_active":true,"members":{"$elemMatch":{"user_service_id":&p.contract.binding.connection_id,"enabled":true}}},
        )),
        ProposalSource::Native { policy_id, revision } => Some((
            crate::models::permission_policy::COLLECTION_NAME,
            doc! {"_id":policy_id,"user_id":&row.user_id,"revision":*revision,"paused":false},
        )),
        ProposalSource::Voice { request_id } => Some((
            crate::models::assistant_voice::REQUESTS,
            doc! {"_id":request_id,"user_id":&row.user_id,"execution_enrolled":true,"state":{"$ne":"cancelled"},"task_conversation_id":&p.conversation_id},
        )),
        ProposalSource::Watch { watch_id, operation_id, .. } => {
            let watch=state.db.collection::<crate::models::async_service_operation::AsyncServiceOperation>("assistant_async_operations")
                .find_one(doc! {"_id":watch_id,"user_id":&row.user_id,"api_key_id":&p.key_id,"expires_at":{"$gt":bson::DateTime::now()}}).await?.ok_or_else(missing)?;
            if crate::services::cma_operation::poll_id(&watch)!=Some(operation_id.as_str()) || watch.state=="cancelled" {return Err(missing());}
            None
        }
    };
    if let Some((collection, filter)) = binding
        && state
            .db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .await?
            .is_none()
    {
        return Err(missing());
    }
    let result = decrypt_result(state, row).await?;
    Ok(ExecutionView {
        version: 2,
        state: p.state.clone(),
        failure_code: p.failure_code.clone(),
        result_available: result.is_some(),
        result,
    })
}

pub(crate) async fn decrypt_result(
    state: &crate::AppState,
    row: &ApprovalRequest,
) -> AppResult<Option<ExecutionResult>> {
    let p = proposal(row)?;
    if p.delivery_abandoned || row.created_at < Utc::now() - Duration::days(1) {
        return Ok(None);
    }
    let Some(value) = &p.result else {
        return Ok(None);
    };
    let raw = Zeroizing::new(
        state
            .encryption_keys
            .decrypt_with_aad(&value.bytes, format!("{}:result", aad(&row.id)).as_bytes())
            .await?,
    );
    if raw.len() > MAX_PAYLOAD {
        return Err(conflict());
    }
    serde_json::from_slice(&raw).map(Some).map_err(encoding)
}

/// Called only by the verified node gateway, after its signed job/runtime/lease checks.
pub(crate) async fn machine_readback(
    state: &crate::AppState,
    auth: &crate::mw::auth::AuthUser,
    job: &crate::models::machine_job::MachineJob,
    id: &str,
) -> AppResult<ExecutionView> {
    let row=state.db.collection::<ApprovalRequest>(REQUESTS).find_one(doc!{"_id":id,"user_id":&job.user_id,"execution_proposal.version":2,"execution_proposal.v2.source.kind":"machine","execution_proposal.v2.source.binding.job_id":&job.id,"execution_proposal.v2.source.binding.node_id":&job.node_id,"execution_proposal.v2.source.binding.runtime_id":&job.runtime_id}).await?.ok_or_else(missing)?;
    let p = proposal(&row)?;
    let ProposalSource::Machine { binding } = &p.source else {
        return Err(missing());
    };
    binding
        .job(&state.db)
        .await
        .map_err(|e| if e.is_forbidden() { missing() } else { e })?;
    let context = auth.execution_permission.as_ref().ok_or_else(missing)?;
    context
        .machine
        .set(Arc::new(machine::Admission {
            binding: (**binding).clone(),
            deadline: row.expires_at,
            job_deadline: job.expires_at,
            cancellation: Default::default(),
        }))
        .map_err(|_| missing())?;
    readback(state, auth, &row).await
}
