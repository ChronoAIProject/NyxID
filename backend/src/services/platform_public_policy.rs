//! Per-manager persisted deployment verification; no process-global authority.
use super::{api_key_mutation_service as tx, platform_manager_contract::*};
use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_public_contract as stored,
        platform_public_contract::{DEPLOYMENT_RECORDS, PublicDeploymentRecord},
    },
};
use chrono::Utc;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use sha2::{Digest, Sha256};

pub(crate) struct VerifiedPublicContract {
    policy_digest: String,
}
impl VerifiedPublicContract {
    pub(crate) fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    #[cfg(test)]
    pub(crate) fn for_local_tests(policy_digest: String) -> Self {
        Self { policy_digest }
    }
}

pub(crate) fn unavailable() -> AppError {
    AppError::Conflict(
        "Public runtime unavailable; validate the deployed public contract before enabling".into(),
    )
}

pub fn candidate_digest(policy: &ManagerRuntimePolicy) -> AppResult<String> {
    let selection = policy.public_policy.as_ref().ok_or_else(unavailable)?;
    super::platform_entry_scope::validate(
        &policy.manager.entry,
        &policy.manager.manager_bot_id,
        None,
    )?;
    let (domain, identity) = match &policy.manager.entry {
        EntryIdentity::TelegramV1 { telegram_bot_id } => (
            "nyxid-public-candidate.v1",
            serde_json::json!(telegram_bot_id),
        ),
        EntryIdentity::NativeV2 { platform_entry } => (
            "nyxid-public-candidate.v2",
            serde_json::to_value(platform_entry).map_err(|_| unavailable())?,
        ),
    };
    Ok(super::mcp_service::canonical_sha256(serde_json::json!([
        domain,
        policy.manager.profile_id,
        policy.manager.manager_bot_id,
        identity,
        policy.manager.manager_authority_epoch,
        policy.public_policy_revision,
        selection.sponsor_owner_user_id,
        selection.nyxagent_profile,
        selection.total_daily_credits.to_string(),
        selection.sender_daily_credits.to_string(),
    ])))
}

pub(crate) fn binding(policy: &ManagerRuntimePolicy) -> AppResult<stored::ManagerBinding> {
    Ok(stored::ManagerBinding {
        profile_id: policy.manager.profile_id.clone(),
        manager_bot_id: policy.manager.manager_bot_id.clone(),
        entry: policy.manager.entry.clone(),
        manager_authority_epoch: policy.manager.manager_authority_epoch,
        public_policy_revision: policy.public_policy_revision,
        sponsor_owner_user_id: policy
            .public_policy
            .as_ref()
            .ok_or_else(unavailable)?
            .sponsor_owner_user_id
            .clone(),
        candidate_digest: candidate_digest(policy)?,
    })
}

pub(crate) fn digest(value: &impl serde::Serialize) -> AppResult<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(value).map_err(|_| unavailable())?,
    )))
}

pub(crate) fn policy_digest(e: &stored::DeploymentEvidence) -> AppResult<String> {
    digest(&(
        &e.binding,
        &e.manifest,
        &e.selected_profile,
        &e.nyxagent_service_id,
        &e.nyxagent_catalog_digest,
    ))
}
pub(crate) fn evidence_digest(e: &stored::DeploymentEvidence) -> AppResult<String> {
    digest(&(
        &e.policy_digest,
        &e.probe_id,
        e.expires_at.timestamp_millis(),
    ))
}

pub(crate) fn operation(op: &stored::ModelOperation) -> AppResult<PublicModelOperation> {
    Ok(PublicModelOperation {
        reasoning_effort: op.reasoning_effort.clone(),
        service_id: op.service_id.clone(),
        operation: op.operation.clone(),
        method: http::Method::from_bytes(op.method.as_bytes()).map_err(|_| unavailable())?,
        canonical_path: op.canonical_path.clone(),
        protocol: match op.protocol.as_str() {
            "openai_responses" => PublicModelProtocol::OpenAiResponses,
            "openai_completions" => PublicModelProtocol::OpenAiCompletions,
            "anthropic_messages" => PublicModelProtocol::AnthropicMessages,
            _ => return Err(unavailable()),
        },
        execution_authority_digest: op.execution_authority_digest.clone(),
        endpoint_generation: (op.endpoint_generation > 0).then_some(op.endpoint_generation),
        model_id: op.model_id.clone(),
        max_input_bytes: op.max_input_bytes,
        max_input_tokens: op.max_input_tokens,
        max_output_tokens: op.max_output_tokens,
        max_response_bytes: op.max_response_bytes,
    })
}
fn limits(l: &stored::ExecutionLimits) -> PublicExecutionLimits {
    PublicExecutionLimits {
        max_duration_secs: l.max_duration_secs,
        max_model_calls: l.max_model_calls,
        max_tool_calls: l.max_tool_calls,
        max_continuations: l.max_continuations,
        max_input_tokens: l.max_input_tokens,
        max_output_tokens: l.max_output_tokens,
        max_input_bytes: l.max_input_bytes,
        max_context_bytes: l.max_context_bytes,
        max_output_bytes: l.max_output_bytes,
    }
}
fn evidence(record: &PublicDeploymentRecord) -> AppResult<PublicReadinessEvidence> {
    let e = record.evidence.as_ref().ok_or_else(unavailable)?;
    super::platform_public_contract_service::validate_manifest(&e.manifest)?;
    if record.status != "ready"
        || record.profile_id != e.binding.profile_id
        || record.public_policy_revision != e.binding.public_policy_revision
        || record.candidate_digest != e.binding.candidate_digest
        || record.checked_at > Utc::now()
        || e.expires_at <= Utc::now()
        || e.expires_at > record.checked_at + chrono::Duration::minutes(5)
        || policy_digest(e)? != e.policy_digest
        || evidence_digest(e)? != e.evidence_digest
    {
        return Err(unavailable());
    }
    let profile = e
        .manifest
        .profiles
        .iter()
        .find(|p| p.id == e.selected_profile)
        .ok_or_else(unavailable)?;
    Ok(PublicReadinessEvidence {
        manager: ManagerAuthority {
            profile_id: e.binding.profile_id.clone(),
            manager_bot_id: e.binding.manager_bot_id.clone(),
            entry: e.binding.entry.clone(),
            manager_authority_epoch: e.binding.manager_authority_epoch,
        },
        public_policy_revision: e.binding.public_policy_revision,
        policy_digest: e.policy_digest.clone(),
        proof_record_id: record.id.clone(),
        candidate_digest: record.candidate_digest.clone(),
        evidence_digest: e.evidence_digest.clone(),
        sponsor_owner_user_id: e.binding.sponsor_owner_user_id.clone(),
        nyxagent_build: e.manifest.build.clone(),
        nyxagent_profile: profile.id.clone(),
        model_operations: profile
            .model_operations
            .iter()
            .map(operation)
            .collect::<AppResult<_>>()?,
        limits: limits(&profile.limits),
        checked_at: record.checked_at,
        expires_at: e.expires_at,
    })
}

/// Candidate proof is selected by exact manager and policy, including prices,
/// catalog destinations, credentials and optional endpoint generations. This
/// write conflicts with proof replacement and live catalog mutations.
pub(crate) async fn fence_readiness(
    db: &Database,
    session: &mut ClientSession,
    reviewed: &ManagerRuntimePolicy,
    expected: Option<&PublicReadinessEvidence>,
) -> AppResult<(VerifiedPublicContract, PublicReadinessEvidence)> {
    let binding = binding(reviewed)?;
    let mut filter = doc! {"candidate_digest":&binding.candidate_digest,"status":"ready",
    "evidence.binding":bson::to_bson(&binding).map_err(|_|unavailable())?,
    "$expr":{"$gt":["$evidence.expires_at","$$NOW"]}};
    if let Some(expected) = expected {
        filter.insert("_id", &expected.proof_record_id);
        filter.insert("evidence.evidence_digest", &expected.evidence_digest);
    }
    let record = db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .find_one_and_update(filter, doc! {"$inc":{"effect_revision":1_i64}})
        .sort(doc! {"checked_at":-1,"_id":-1})
        .session(&mut *session)
        .await?
        .ok_or_else(unavailable)?;
    let proof = evidence(&record)?;
    let deployed = record.evidence.as_ref().ok_or_else(unavailable)?;
    if deployed.selected_profile
        != reviewed
            .public_policy
            .as_ref()
            .ok_or_else(unavailable)?
            .nyxagent_profile
    {
        return Err(unavailable());
    }
    super::platform_public_contract_service::fence_deployment(db, session, deployed).await?;
    let observed = db.collection::<stored::PublicProbe>(stored::PROBES)
        .update_one(doc! {
            "_id": &deployed.probe_id, "state": "settled", "exact_model_observed": true,
            "binding": bson::to_bson(&deployed.binding).map_err(|_| unavailable())?,
            "manifest": bson::to_bson(&deployed.manifest).map_err(|_| unavailable())?,
            "selected_profile": &deployed.selected_profile, "nyxagent_service_id": &deployed.nyxagent_service_id,
            "nyxagent_catalog_digest": &deployed.nyxagent_catalog_digest,
        }, doc! {"$inc": {"proof_fence": 1_i64}})
        .session(&mut *session).await?;
    if observed.matched_count != 1 {
        return Err(unavailable());
    }

    if let Some(expected) = expected
        && (proof.policy_digest != expected.policy_digest
            || proof.manager != expected.manager
            || proof.public_policy_revision != expected.public_policy_revision
            || proof.candidate_digest != expected.candidate_digest
            || proof.nyxagent_build != expected.nyxagent_build
            || proof.nyxagent_profile != expected.nyxagent_profile
            || proof.sponsor_owner_user_id != expected.sponsor_owner_user_id
            || proof.checked_at != expected.checked_at
            || proof.expires_at != expected.expires_at
            || proof.model_operations.len() != expected.model_operations.len()
            || !proof
                .model_operations
                .iter()
                .zip(&expected.model_operations)
                .all(|(a, b)| operation_matches(a, b))
            || !limits_match(&proof.limits, &expected.limits))
    {
        return Err(unavailable());
    }
    Ok((
        VerifiedPublicContract {
            policy_digest: proof.policy_digest.clone(),
        },
        proof,
    ))
}

pub(crate) async fn verified_readiness_in_session(
    db: &Database,
    session: &mut ClientSession,
    manager: &ManagerAuthority,
    revision: i64,
    original_digest: Option<&str>,
    proof_id: Option<&str>,
) -> AppResult<(VerifiedPublicContract, PublicReadinessEvidence)> {
    let row = super::platform_bot_runtime_control_service::fence_manager_in_session(
        db,
        session,
        &manager.profile_id,
        &manager.manager_bot_id,
        &manager.entry,
        None,
    )
    .await?
    .ok_or_else(unavailable)?;
    let reviewed = super::platform_bot_runtime_control_service::policy(&row);
    if reviewed.manager != *manager
        || reviewed.public_policy_revision != revision
        || !reviewed.public_enabled
        || reviewed.emergency_stopped
        || row.retired
    {
        return Err(unavailable());
    }
    let active = db
        .collection::<stored::PublicDeploymentActivation>(stored::ACTIVATIONS)
        .find_one_and_update(
            doc! {"_id": &manager.profile_id,
            "binding": bson::to_bson(&binding(&reviewed)?).map_err(|_| unavailable())?},
            doc! {"$inc": {"effect_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(unavailable)?;
    if original_digest.is_some_and(|digest| digest != active.policy_digest) {
        return Err(unavailable());
    }
    let expected = if let Some(id) = proof_id {
        Some(evidence(
            &db.collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
                .find_one(doc! {"_id":id})
                .session(&mut *session)
                .await?
                .ok_or_else(unavailable)?,
        )?)
    } else {
        None
    };
    let expected = match expected {
        Some(proof) => proof,
        None => {
            let record = db
                .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
                .find_one(doc! {"candidate_digest": &active.binding.candidate_digest,
                "evidence.policy_digest": &active.policy_digest, "status": "ready"})
                .sort(doc! {"checked_at": -1, "_id": -1})
                .session(&mut *session)
                .await?
                .ok_or_else(unavailable)?;
            evidence(&record)?
        }
    };
    let result = fence_readiness(db, session, &reviewed, Some(&expected)).await?;
    if result.1.policy_digest != active.policy_digest {
        return Err(unavailable());
    }
    let controls = db
        .collection::<mongodb::bson::Document>(
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
        )
        .update_one(
            doc! {
                "_id": &row.id, "manager_authority_epoch": manager.manager_authority_epoch,
                "public_policy_revision": revision, "public_enabled": true,
                "emergency_stopped": false, "retired": false,
            },
            doc! {"$inc": {"effect_revision": 1_i64}},
        )
        .session(session)
        .await?;
    if controls.matched_count != 1 {
        return Err(unavailable());
    }
    Ok(result)
}

pub(crate) async fn verified_readiness(
    db: &Database,
    manager: &ManagerAuthority,
    revision: i64,
    original_digest: Option<&str>,
    proof_id: Option<&str>,
) -> AppResult<(VerifiedPublicContract, PublicReadinessEvidence)> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (
                db.clone(),
                manager.clone(),
                original_digest.map(str::to_owned),
                proof_id.map(str::to_owned),
            ),
            move |session, (db, manager, digest, proof_id)| {
                Box::pin(async move {
                    tx::transaction_result(
                        verified_readiness_in_session(
                            db,
                            session,
                            manager,
                            revision,
                            digest.as_deref(),
                            proof_id.as_deref(),
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)
}

pub(crate) async fn verified_context_in_session(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
) -> AppResult<VerifiedPublicContract> {
    let (proof, e) = verified_readiness_in_session(
        db,
        session,
        &context.ingress.manager,
        context.ingress.public_policy_revision,
        Some(&context.policy_digest),
        Some(&context.proof_record_id),
    )
    .await?;
    check_context(context, &e)?;
    Ok(proof)
}
pub(crate) async fn verified_context(
    db: &Database,
    context: &PublicExecutionContext,
) -> AppResult<VerifiedPublicContract> {
    let (proof, e) = verified_readiness(
        db,
        &context.ingress.manager,
        context.ingress.public_policy_revision,
        Some(&context.policy_digest),
        Some(&context.proof_record_id),
    )
    .await?;
    check_context(context, &e)?;
    Ok(proof)
}
fn check_context(c: &PublicExecutionContext, e: &PublicReadinessEvidence) -> AppResult<()> {
    if c.sponsor_owner_user_id != e.sponsor_owner_user_id
        || c.nyxagent_profile != e.nyxagent_profile
        || c.expires_at > e.expires_at
        || c.expires_at <= Utc::now()
        || c.model_operations.len() != e.model_operations.len()
        || !c
            .model_operations
            .iter()
            .zip(&e.model_operations)
            .all(|(a, b)| operation_matches(a, b))
        || !limits_match(&c.limits, &e.limits)
    {
        return Err(unavailable());
    }
    Ok(())
}

pub(crate) fn operation_matches(a: &PublicModelOperation, b: &PublicModelOperation) -> bool {
    a.service_id == b.service_id
        && a.operation == b.operation
        && a.method == b.method
        && a.canonical_path == b.canonical_path
        && a.protocol == b.protocol
        && a.execution_authority_digest == b.execution_authority_digest
        && a.endpoint_generation == b.endpoint_generation
        && a.model_id == b.model_id
        && a.reasoning_effort == b.reasoning_effort
        && a.max_input_bytes == b.max_input_bytes
        && a.max_input_tokens == b.max_input_tokens
        && a.max_output_tokens == b.max_output_tokens
        && a.max_response_bytes == b.max_response_bytes
}
fn limits_match(a: &PublicExecutionLimits, b: &PublicExecutionLimits) -> bool {
    a.max_duration_secs == b.max_duration_secs
        && a.max_model_calls == b.max_model_calls
        && a.max_tool_calls == b.max_tool_calls
        && a.max_continuations == b.max_continuations
        && a.max_input_tokens == b.max_input_tokens
        && a.max_output_tokens == b.max_output_tokens
        && a.max_input_bytes == b.max_input_bytes
        && a.max_context_bytes == b.max_context_bytes
        && a.max_output_bytes == b.max_output_bytes
}

/// Runtime status reads the same persisted, live-scoped proof used by Enable.
/// Infrastructure failures propagate; an absent/expired proof is unavailable.
pub async fn assess(db: &Database, reviewed: &ManagerRuntimePolicy) -> AppResult<PublicReadiness> {
    let digest = candidate_digest(reviewed)?;
    let record = db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .find_one(doc! {"candidate_digest":digest,"status":"ready"})
        .sort(doc! {"checked_at":-1,"_id":-1})
        .await?;
    let Some(record) = record else {
        return Ok(PublicReadiness::Unavailable(
            UnavailableReason::PublicNotReady,
        ));
    };
    assess_record(db, reviewed, &record).await
}

pub(crate) async fn assess_record(
    db: &Database,
    reviewed: &ManagerRuntimePolicy,
    record: &PublicDeploymentRecord,
) -> AppResult<PublicReadiness> {
    if record
        .evidence
        .as_ref()
        .is_some_and(|e| e.expires_at <= Utc::now())
    {
        return Ok(PublicReadiness::Unavailable(
            UnavailableReason::ProofExpired,
        ));
    }
    let expected = match evidence(record) {
        Ok(e) => e,
        Err(_) => {
            return Ok(PublicReadiness::Unavailable(
                UnavailableReason::PublicNotReady,
            ));
        }
    };
    let mut session = db.client().start_session().await?;
    let result = session
        .start_transaction()
        .and_run(
            (db.clone(), reviewed.clone(), expected),
            |session, (db, reviewed, expected)| {
                Box::pin(async move {
                    tx::transaction_result(
                        fence_readiness(db, session, reviewed, Some(expected)).await,
                    )
                })
            },
        )
        .await;
    match result.map_err(tx::map_transaction_error) {
        Ok((_, e)) => Ok(PublicReadiness::Ready(e)),
        Err(AppError::Conflict(_) | AppError::Forbidden(_)) => Ok(PublicReadiness::Unavailable(
            UnavailableReason::PublicNotReady,
        )),
        Err(error) => Err(error),
    }
}

pub(crate) async fn activation(
    db: &Database,
    reviewed: &ManagerRuntimePolicy,
) -> AppResult<stored::PublicDeploymentActivation> {
    db.collection::<stored::PublicDeploymentActivation>(stored::ACTIVATIONS)
        .find_one(doc! {"_id": &reviewed.manager.profile_id,
        "binding": bson::to_bson(&binding(reviewed)?).map_err(|_| unavailable())?})
        .await?
        .ok_or_else(unavailable)
}

pub async fn assess_enabled(
    db: &Database,
    reviewed: &ManagerRuntimePolicy,
) -> AppResult<PublicReadiness> {
    let active = match activation(db, reviewed).await {
        Ok(active) => active,
        Err(AppError::Conflict(_) | AppError::Forbidden(_)) => {
            return Ok(PublicReadiness::Unavailable(
                UnavailableReason::PublicNotReady,
            ));
        }
        Err(error) => return Err(error),
    };
    let record = db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .find_one(doc! {"candidate_digest": &active.binding.candidate_digest,
        "evidence.policy_digest": &active.policy_digest, "status": "ready"})
        .sort(doc! {"checked_at": -1, "_id": -1})
        .await?;
    let Some(record) = record else {
        return Ok(PublicReadiness::Unavailable(
            UnavailableReason::PublicNotReady,
        ));
    };
    match assess_record(db, reviewed, &record).await? {
        PublicReadiness::Unavailable(reason) => Ok(PublicReadiness::Unavailable(reason)),
        PublicReadiness::Ready(evidence) => match verified_readiness(
            db,
            &reviewed.manager,
            reviewed.public_policy_revision,
            Some(&active.policy_digest),
            Some(&evidence.proof_record_id),
        )
        .await
        {
            Ok((_, e)) => Ok(PublicReadiness::Ready(e)),
            Err(AppError::Conflict(_) | AppError::Forbidden(_)) => Ok(
                PublicReadiness::Unavailable(UnavailableReason::PublicNotReady),
            ),
            Err(e) => Err(e),
        },
    }
}
