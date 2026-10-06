//! Human review and a one-card, fenced publish-and-pin saga. Proposal content
//! stays encrypted at rest; no text or provider errors enter audit or Debug.
use super::{
    agent_skill_service::{self as skills, OrnnReader, Selection},
    api_key_mutation_service as transactions,
    assistant_acknowledgement_service::{self as acks, ChatAuthority},
    assistant_agent_learning::{self as learning, GeneratedProposal},
    assistant_learning_publication as publication, assistant_team_service as team,
    feature_flag_service, org_agent_service,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_agent::{AgentSkillMetadata, AssistantAgent},
        assistant_agent_learning::*,
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub const TOOL: &str = "nyxid__approve_agent_learning";
const LEASE_SECONDS: i64 = 180;
fn conflict() -> AppError {
    AppError::Conflict("Learning proposal changed; reload and review it again".into())
}
fn not_found() -> AppError {
    AppError::NotFound("Learning proposal not found".into())
}

fn failure_status(proposal: &AssistantAgentLearningProposal) -> &'static str {
    if proposal
        .publication
        .as_ref()
        .is_some_and(|publication| publication.started)
    {
        "publication_failed"
    } else {
        "published_unpinned"
    }
}

fn approval_actor_allowed(agent: &AssistantAgent, actor: &str, chat: &ChatAuthority) -> bool {
    agent.user_id == actor && !chat.guest && chat.is_orchestrator()
}

fn resumes_publication(
    publication: &LearningPublication,
    actor: &str,
    card: &str,
    digest: &str,
) -> bool {
    publication.approved_by.as_deref() == Some(actor)
        && publication.acknowledgement_id.as_deref() == Some(card)
        && publication.approval_digest.as_deref() == Some(digest)
}

pub(super) async fn gate(db: &Database, actor: &str) -> AppResult<()> {
    if !feature_flag_service::personal_flag_enabled(db, actor, learning::FLAG_KEY).await? {
        return Err(AppError::ValidationError(
            "Automatic agent learning is not enabled yet".into(),
        ));
    }
    Ok(())
}

#[derive(Serialize)]
pub struct LearningStatus {
    pub available: bool,
    pub can_maintain: bool,
    pub org_owned: bool,
    pub enabled: bool,
    pub threshold: i64,
    pub learning_epoch: i64,
    pub config_revision: i64,
    pub eligible_count: i64,
    pub last_run_at: Option<chrono::DateTime<Utc>>,
    pub last_error_code: Option<String>,
    pub opted_in: bool,
    pub publication_blocked: Option<&'static str>,
}
pub async fn status(db: &Database, actor: &str, agent_id: &str) -> AppResult<LearningStatus> {
    let agent = team::agent(db, actor, agent_id).await?;
    let access = org_agent_service::access(db, actor, &agent.user_id).await?;
    let available =
        feature_flag_service::personal_flag_enabled(db, actor, learning::FLAG_KEY).await?;
    let config = if available {
        db.collection::<AssistantAgentLearning>(CONFIG_COLLECTION_NAME)
            .find_one(doc! {"_id":agent_id,"owner_id":&agent.user_id})
            .await?
    } else {
        None
    };
    let member = if available && actor != agent.user_id {
        db.collection::<AssistantAgentLearningMember>(MEMBERS_COLLECTION_NAME)
            .find_one(doc! {"agent_id":agent_id,"member_user_id":actor})
            .await?
    } else {
        None
    };
    Ok(LearningStatus {
        available,
        can_maintain: agent.destroyed_at.is_none() && org_agent_service::can_maintain(&access),
        org_owned: actor != agent.user_id,
        enabled: config.as_ref().is_some_and(|c| c.enabled),
        threshold: config.as_ref().map_or(DEFAULT_THRESHOLD, |c| c.threshold),
        learning_epoch: config.as_ref().map_or(0, |c| c.learning_epoch),
        config_revision: config.as_ref().map_or(0, |c| c.config_revision),
        eligible_count: config.as_ref().map_or(0, |c| c.eligible_count),
        last_run_at: config.as_ref().and_then(|c| c.last_run_at),
        last_error_code: config.as_ref().and_then(|c| c.last_error_code.clone()),
        opted_in: member.as_ref().is_some_and(|m| {
            m.opted_in
                && config
                    .as_ref()
                    .is_some_and(|c| m.learning_epoch == c.learning_epoch)
        }),
        publication_blocked: (actor != agent.user_id).then_some("owner_binding_unavailable"),
    })
}

#[derive(Serialize)]
pub struct EvidenceItem {
    pub label: String,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
}
#[derive(Serialize)]
pub struct ProposalItem {
    pub id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub run_id: String,
    pub status: String,
    pub revision: i64,
    pub config_revision: i64,
    pub agent_skills_revision: i64,
    pub current_skills_revision: i64,
    pub model_contract: String,
    pub evidence_count: usize,
    pub body_bytes: i64,
    pub created_at: chrono::DateTime<Utc>,
    pub failure_code: Option<String>,
    pub evidence: Vec<EvidenceItem>,
    pub draft: Option<GeneratedProposal>,
    pub published_skill_id: Option<String>,
    pub published_version: Option<String>,
}

async fn load(
    db: &Database,
    actor: &str,
    agent_id: &str,
    id: &str,
) -> AppResult<(AssistantAgent, AssistantAgentLearningProposal)> {
    gate(db, actor).await?;
    let agent = team::maintained_agent(db, actor, agent_id).await?;
    if agent.destroyed_at.is_some() {
        return Err(not_found());
    }
    let row = db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":id,"agent_id":agent_id,"owner_id":&agent.user_id})
        .await?
        .ok_or_else(not_found)?;
    Ok((agent, row))
}

async fn current(
    db: &Database,
    agent: &AssistantAgent,
    row: &AssistantAgentLearningProposal,
) -> AppResult<bool> {
    let config = db.collection::<AssistantAgentLearning>(CONFIG_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.agent_id,"owner_id":&row.owner_id,"enabled":true,"config_revision":row.config_revision}).await?;
    let Some(config) = config else {
        return Ok(false);
    };
    let run = db.collection::<AssistantAgentLearningRun>(RUNS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.run_id,"agent_id":&row.agent_id,"owner_id":&row.owner_id,"status":"succeeded","learning_epoch":config.learning_epoch}).await?;
    let Some(mut run) = run else {
        return Ok(false);
    };
    if row.evidence.is_empty() {
        return Ok(false);
    }
    run.evidence = row.evidence.clone();
    learning::all_evidence_current(db, &run, agent).await
}
async fn require_current(
    db: &Database,
    agent: &AssistantAgent,
    row: &AssistantAgentLearningProposal,
) -> AppResult<()> {
    if !current(db, agent, row).await? {
        db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(
            doc! {"_id":&row.id,"revision":revision_filter(row.revision),"status":{"$nin":["pinned","rejected"]}},
            doc! {"$set":{"status":"invalidated","failure_code":"evidence_unavailable","body_bytes":0,"updated_at":bson::DateTime::now()},"$unset":{"body_encrypted":""}},
        ).await?;
        return Err(AppError::Conflict(
            "Learning evidence or consent is no longer available".into(),
        ));
    }
    Ok(())
}
fn revision_filter(revision: i64) -> bson::Bson {
    if revision == 0 {
        bson::to_bson(&doc! {"$in":[bson::Bson::Null,bson::Bson::Int64(0)]}).expect("static BSON")
    } else {
        revision.into()
    }
}
async fn draft(
    state: &AppState,
    row: &AssistantAgentLearningProposal,
) -> AppResult<GeneratedProposal> {
    let bytes = state
        .encryption_keys
        .decrypt(&row.body_encrypted)
        .await
        .map_err(|_| not_found())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| not_found())?;
    let valid = learning::validate_generated(text)?.ok_or_else(not_found)?;
    serde_json::from_slice(&valid).map_err(|_| not_found())
}

/// Improvements may only target an active L1 root at its exact current B2 pin.
/// Called after model validation, on edit, and before every publication/pin.
pub(crate) async fn validate_base(
    db: &Database,
    agent: &AssistantAgent,
    draft: &GeneratedProposal,
) -> AppResult<()> {
    let Some(base) = &draft.base_skill else {
        return Ok(());
    };
    let pin = agent.skills.iter().find(|s| {
        s.source == base.source
            && s.skill_id == base.skill_id
            && s.version == base.version
            && s.sha256 == base.sha256
            && s.name == base.name
            && s.dependencies.is_empty()
    });
    if base.source != "ornn" || pin.is_none() || db.collection::<AssistantAgentLearningSkillRoot>(ROOTS_COLLECTION_NAME)
        .find_one(doc! {"agent_id":&agent.id,"owner_id":&agent.user_id,"skill_id":&base.skill_id,"version":&base.version,"sha256":&base.sha256}).await?.is_none() {
        return Err(AppError::Conflict("base_skill_changed".into()));
    }
    publication::next_version(Some(base))?;
    Ok(())
}

pub async fn list(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    include_drafts: bool,
) -> AppResult<Vec<ProposalItem>> {
    gate(&state.db, actor).await?;
    let agent = team::maintained_agent(&state.db, actor, agent_id).await?;
    let rows: Vec<AssistantAgentLearningProposal> = state.db.collection(PROPOSALS_COLLECTION_NAME)
        .find(doc! {"agent_id":agent_id,"owner_id":&agent.user_id,"status":{"$nin":["rejected","invalidated"]}})
        .sort(doc! {"created_at":-1}).limit(16).await?.try_collect().await?;
    let mut result = Vec::new();
    for row in rows {
        let valid = row.status == "pinned" || current(&state.db, &agent, &row).await?;
        if !valid {
            require_current(&state.db, &agent, &row).await.ok();
            continue;
        }
        let body = if include_drafts && row.status != "pinned" {
            Some(draft(state, &row).await?)
        } else {
            None
        };
        let evidence = if include_drafts {
            row.evidence
                .iter()
                .map(|e| EvidenceItem {
                    label: e.label.clone(),
                    conversation_id: (e.principal_id == actor).then(|| e.conversation_id.clone()),
                    turn_id: (e.principal_id == actor).then(|| e.turn_id.clone()),
                })
                .collect()
        } else {
            Vec::new()
        };
        result.push(ProposalItem {
            id: row.id,
            agent_id: row.agent_id,
            owner_id: row.owner_id,
            run_id: row.run_id,
            status: row.status,
            revision: row.revision,
            config_revision: row.config_revision,
            agent_skills_revision: row.agent_skills_revision,
            current_skills_revision: agent.skills_revision,
            model_contract: row.model_contract,
            evidence_count: row.evidence.len(),
            body_bytes: row.body_bytes,
            created_at: row.created_at,
            failure_code: row.failure_code,
            evidence,
            draft: body,
            published_skill_id: row.publication.as_ref().and_then(|p| p.skill_id.clone()),
            published_version: row.publication.as_ref().map(|p| p.version.clone()),
        });
    }
    Ok(result)
}

async fn audit(db: &Database, actor: &str, row: &AssistantAgentLearningProposal, event: &str) {
    let _ = super::audit_service::log_actor_event(db.clone(), &super::audit_service::AuditActor {user_id:actor.into(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},event,
        Some(json!({"agent_id":row.agent_id,"owner_id":row.owner_id,"proposal_id":row.id,"revision":row.revision,"evidence_count":row.evidence.len()}))).await;
}

pub async fn edit(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    id: &str,
    expected_revision: i64,
    value: &Value,
) -> AppResult<()> {
    let (agent, row) = load(&state.db, actor, agent_id, id).await?;
    require_current(&state.db, &agent, &row).await?;
    if row.revision != expected_revision || row.status != "pending" {
        return Err(conflict());
    }
    let encoded = learning::validate_generated(&value.to_string())?.ok_or_else(conflict)?;
    let value: GeneratedProposal = serde_json::from_slice(&encoded).map_err(|_| conflict())?;
    validate_base(&state.db, &agent, &value).await?;
    let encrypted = state.encryption_keys.encrypt(&encoded).await?;
    let fingerprint = super::assistant_action_receipts::fingerprint_sensitive_material(&format!(
        "{}:{}:{}",
        agent.id,
        row.model_contract,
        String::from_utf8_lossy(&encoded)
    ));
    let changed = state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(
        doc! {"_id":id,"status":"pending","revision":revision_filter(expected_revision)},
        doc! {"$set":{"body_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},"body_bytes":encoded.len() as i64,"fingerprint":fingerprint,"revision":expected_revision+1,"agent_skills_revision":agent.skills_revision,"updated_at":bson::DateTime::now()},"$unset":{"publication":"","failure_code":""}},
    ).await?;
    if changed.modified_count != 1 {
        return Err(conflict());
    }
    audit(&state.db, actor, &row, "assistant_learning_proposal_edited").await;
    Ok(())
}

pub async fn reject(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    id: &str,
    revision: i64,
    reason: &str,
) -> AppResult<()> {
    if !matches!(
        reason,
        "rejected" | "not_useful" | "contains_private_material" | "unsafe_guidance"
    ) {
        return Err(AppError::ValidationError(
            "Choose a valid rejection reason".into(),
        ));
    }
    let (_, row) = load(&state.db, actor, agent_id, id).await?;
    let db = state.db.clone();
    let proposal_id = id.to_owned();
    let agent_id = agent_id.to_owned();
    let fingerprint = row.fingerprint.clone();
    let reason = reason.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let changed = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&proposal_id,"status":"pending","revision":revision_filter(revision)},
                        doc! {"$set":{"status":"rejected","body_bytes":0,"failure_code":&reason,"updated_at":bson::DateTime::now()},"$unset":{"body_encrypted":""}},
                    )
                    .session(&mut *session)
                    .await?;
                if changed.modified_count != 1 {
                    return Err(conflict());
                }
                let now = Utc::now();
                db.collection::<bson::Document>(REJECTIONS_COLLECTION_NAME)
                    .update_one(
                        doc! {"agent_id":&agent_id,"fingerprint":&fingerprint},
                        doc! {"$set":{"expires_at":bson::DateTime::from_chrono(now+Duration::days(REJECTION_TTL_DAYS)),"created_at":bson::DateTime::from_chrono(now)},"$setOnInsert":{"_id":Uuid::new_v4().to_string()}},
                    )
                    .upsert(true)
                    .session(&mut *session)
                    .await?;
                let mut cursor = db
                    .collection::<AssistantAgentLearningRejection>(REJECTIONS_COLLECTION_NAME)
                    .find(doc! {"agent_id":&agent_id})
                    .sort(doc! {"created_at":-1,"_id":-1})
                    .skip(MAX_REJECTIONS_PER_AGENT as u64)
                    .session(&mut *session)
                    .await?;
                let old: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
                if !old.is_empty() {
                    db.collection::<bson::Document>(REJECTIONS_COLLECTION_NAME)
                        .delete_many(doc! {"_id":{"$in":old.iter().map(|r| &r.id).collect::<Vec<_>>()}})
                        .session(&mut *session)
                        .await?;
                }
                Ok(())
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    audit(
        &state.db,
        actor,
        &row,
        "assistant_learning_proposal_rejected",
    )
    .await;
    Ok(())
}

/// Allocate immutable external operation metadata before showing the card.
/// Binding uses the live B2 revision; retries after a B2 edit need a new card,
/// but they reuse the same external operation and cannot repeat its write.
pub async fn approval_binding(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    id: &str,
    expected_revision: i64,
    expected_skills_revision: i64,
) -> AppResult<Value> {
    let (agent, mut row) = load(&state.db, actor, agent_id, id).await?;
    if agent.user_id != actor {
        return Err(AppError::Conflict("owner_binding_unavailable".into()));
    }
    if row.revision != expected_revision
        || agent.skills_revision != expected_skills_revision
        || !matches!(
            row.status.as_str(),
            "pending" | "publication_failed" | "published_unpinned" | "publishing"
        )
    {
        return Err(conflict());
    }
    require_current(&state.db, &agent, &row).await?;
    let body = draft(state, &row).await?;
    validate_base(&state.db, &agent, &body).await?;
    if row.publication.is_none() {
        let operation = Uuid::new_v4().to_string();
        let name = body.base_skill.as_ref().map_or_else(
            || format!("nyx-learning-{}", operation.replace('-', "")),
            |b| b.name.clone(),
        );
        let version = publication::next_version(body.base_skill.as_ref())?;
        let bytes = publication::package(&body, &operation, &name, &version)?;
        let p = LearningPublication {
            operation_id: operation,
            name,
            version,
            sha256: publication::hash(&bytes),
            skill_id: None,
            started: false,
            approved_by: None,
            approval_digest: None,
            acknowledgement_id: None,
            skills_revision: agent.skills_revision,
            lease_id: None,
            lease_expires_at: None,
        };
        state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(doc! {"_id":id,"revision":revision_filter(row.revision),"status":"pending","publication":bson::Bson::Null},doc! {"$set":{"publication":bson::to_bson(&p).map_err(|_| conflict())?}}).await?;
        row = load(&state.db, actor, agent_id, id).await?.1;
    }
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    if p.lease_expires_at.is_some_and(|t| t > Utc::now()) {
        return Err(AppError::Conflict(
            "Publication is in progress; retry when it settles".into(),
        ));
    }
    Ok(publication::binding(&row, p, agent.skills_revision))
}

/// HTTP-only caller has verified first-party human identity and decided this
/// card. Consume and claim in one transaction; a used card can only resume its
/// own durable operation, with identical revisions/digest and approving actor.
pub async fn approve(
    state: &AppState,
    chat: &ChatAuthority,
    agent_id: &str,
    id: &str,
    card: &str,
    binding: &Value,
    reader: &impl OrnnReader,
) -> AppResult<Value> {
    let actor = &chat.user_id;
    let (agent, row) = load(&state.db, actor, agent_id, id).await?;
    if !approval_actor_allowed(&agent, actor, chat) {
        return Err(AppError::Forbidden("owner_binding_unavailable".into()));
    }
    require_current(&state.db, &agent, &row).await?;
    let mut p = row.publication.clone().ok_or_else(conflict)?;
    if publication::binding(&row, &p, agent.skills_revision) != *binding {
        return Err(conflict());
    }
    let digest = acks::arguments_digest(binding);
    let resumed = resumes_publication(&p, actor, card, &digest);
    let lease = Uuid::new_v4().to_string();
    let db = state.db.clone();
    let chat = chat.clone();
    let card = card.to_owned();
    let binding = binding.clone();
    let actor = actor.clone();
    let actor_for_claim = actor.clone();
    let operation_id = p.operation_id.clone();
    let proposal_id = id.to_owned();
    let proposal_revision = row.revision;
    let agent_for_claim = agent.clone();
    let initial = p.clone();
    let mut session = db.client().start_session().await?;
    p = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<LearningPublication> = async {
                if !resumed
                    && !acks::consume_action_in_session(
                        &db,
                        &chat,
                        &card,
                        TOOL,
                        &binding,
                        &mut *session,
                    )
                    .await?
                {
                    return Err(AppError::Conflict(
                        "Learning approval card is missing, expired, used or stale".into(),
                    ));
                }
                let mut next = initial.clone();
                next.approved_by = Some(actor_for_claim.clone());
                next.acknowledgement_id = Some(card.clone());
                next.approval_digest = Some(digest.clone());
                next.skills_revision = agent_for_claim.skills_revision;
                next.lease_id = Some(lease.clone());
                next.lease_expires_at = Some(Utc::now() + Duration::seconds(LEASE_SECONDS));
                let changed = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&proposal_id,"revision":revision_filter(proposal_revision),"publication.operation_id":&operation_id,"status":{"$in":["pending","publishing","publication_failed","published_unpinned"]},"$or":[{"publication.lease_expires_at":bson::Bson::Null},{"publication.lease_expires_at":{"$lte":bson::DateTime::now()}}]},
                        doc! {"$set":{"status":"publishing","publication":bson::to_bson(&next).map_err(|_|conflict())?,"failure_code":bson::Bson::Null,"updated_at":bson::DateTime::now()}},
                    )
                    .session(&mut *session)
                    .await?;
                if changed.modified_count != 1 {
                    return Err(conflict());
                }
                Ok(next)
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    audit(
        &state.db,
        &actor,
        &row,
        "assistant_learning_publication_approved",
    )
    .await;
    let result = Box::pin(execute(state, &actor, &row, &p, reader)).await;
    if result.is_err() {
        let durable = state
            .db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .await?
            .ok_or_else(not_found)?;
        let status = failure_status(&durable);
        state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .update_one(
                doc! {"_id":id,"publication.operation_id":&p.operation_id,"publication.lease_id":&p.lease_id,"status":{"$in":["publishing","published_unpinned"]}},
                doc! {"$set":{"status":status,"failure_code":"publication_retry_required","publication.lease_expires_at":bson::Bson::Null}},
            )
            .await?;
        audit(
            &state.db,
            &actor,
            &row,
            "assistant_learning_publication_deferred",
        )
        .await;
    }
    result
}
fn lease_filter(row: &AssistantAgentLearningProposal, p: &LearningPublication) -> bson::Document {
    doc! {"_id":&row.id,"revision":revision_filter(row.revision),"status":{"$in":["publishing","published_unpinned"]},"publication.operation_id":&p.operation_id,"publication.lease_id":&p.lease_id,"publication.lease_expires_at":{"$gt":bson::DateTime::now()}}
}
async fn recheck(
    state: &AppState,
    actor: &str,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    body: &GeneratedProposal,
) -> AppResult<AssistantAgent> {
    let (agent, latest) = load(&state.db, actor, &row.agent_id, &row.id).await?;
    require_current(&state.db, &agent, &latest).await?;
    if latest.revision != row.revision
        || latest.fingerprint != row.fingerprint
        || agent.skills_revision != p.skills_revision
        || latest.publication.as_ref().is_none_or(|v| {
            v.lease_id != p.lease_id || v.lease_expires_at.is_none_or(|t| t <= Utc::now())
        })
    {
        return Err(conflict());
    }
    validate_base(&state.db, &agent, body).await?;
    Ok(agent)
}
async fn execute(
    state: &AppState,
    actor: &str,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    reader: &impl OrnnReader,
) -> AppResult<Value> {
    let body = draft(state, row).await?;
    recheck(state, actor, row, p, &body).await?;
    let id = if p.started {
        publication::reconcile(reader, actor, p, body.base_skill.as_ref()).await?
    } else {
        let bytes = publication::package(&body, &p.operation_id, &p.name, &p.version)?;
        if publication::hash(&bytes) != p.sha256 {
            return Err(conflict());
        }
        publication::validate(reader, &bytes).await?;
        if let Some(base) = &body.base_skill {
            publication::verify_base(reader, actor, base).await?;
        }
        recheck(state, actor, row, p, &body).await?;
        // Durable uncertainty boundary, before the first potentially effective
        // write. Crash/timeout from here on permits reconciliation reads only.
        let changed = state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .update_one(
                lease_filter(row, p),
                doc! {"$set":{"publication.started":true}},
            )
            .await?;
        if changed.modified_count != 1 {
            return Err(conflict());
        }
        publication::publish(reader, body.base_skill.as_ref(), bytes).await?
    };
    let preview = publication::verify(reader, actor, p, &id).await?;
    let agent = recheck(state, actor, row, p, &body).await?;
    let mut selection = Selection {
        expected_revision: p.skills_revision,
        skills: agent.skills.clone(),
    };
    if let Some(base) = &body.base_skill {
        selection
            .skills
            .retain(|s| s.skill_id != base.skill_id || s.source != base.source);
    }
    selection.skills.push(preview.reference.clone());
    skills::validate(&selection)?;
    let mut metadata = agent.skill_metadata.clone();
    metadata.insert(
        id.clone(),
        AgentSkillMetadata {
            description: preview.description,
            size_bytes: preview.size_bytes,
        },
    );
    let db = state.db.clone();
    let row_for_pin = row.clone();
    let publication_for_pin = p.clone();
    let agent_for_pin = agent.clone();
    let selection_for_pin = selection.clone();
    let metadata_for_pin = metadata.clone();
    let skill_id = id.clone();
    let mut session = db.client().start_session().await?;
    let revision = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<i64> = async {
                let marked = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(
                        lease_filter(&row_for_pin, &publication_for_pin),
                        doc! {"$set":{"status":"published_unpinned","publication.skill_id":&skill_id}},
                    )
                    .session(&mut *session)
                    .await?;
                if marked.matched_count != 1 {
                    return Err(conflict());
                }
                let config = db
                    .collection::<bson::Document>(CONFIG_COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&row_for_pin.agent_id,"owner_id":&row_for_pin.owner_id,"enabled":true,"config_revision":row_for_pin.config_revision},
                        doc! {"$inc":{"publication_fence":1}},
                    )
                    .session(&mut *session)
                    .await?;
                if config.matched_count != 1 {
                    return Err(conflict());
                }
                let completed = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(
                        lease_filter(&row_for_pin, &publication_for_pin),
                        doc! {"$set":{"status":"pinned","failure_code":bson::Bson::Null,"publication.lease_expires_at":bson::Bson::Null,"body_bytes":0,"updated_at":bson::DateTime::now()},"$unset":{"body_encrypted":""}},
                    )
                    .session(&mut *session)
                    .await?;
                if completed.matched_count != 1 {
                    return Err(conflict());
                }
                let revision = skills::apply_verified_in_session(
                    &db,
                    &agent_for_pin,
                    &selection_for_pin,
                    &metadata_for_pin,
                    &mut *session,
                )
                .await?;
                let root = AssistantAgentLearningSkillRoot {
                    id: Uuid::new_v4().to_string(),
                    agent_id: agent_for_pin.id.clone(),
                    owner_id: agent_for_pin.user_id.clone(),
                    skill_id: skill_id.clone(),
                    version: publication_for_pin.version.clone(),
                    sha256: publication_for_pin.sha256.clone(),
                    operation_id: publication_for_pin.operation_id.clone(),
                    active: true,
                    proposal_id: row_for_pin.id.clone(),
                    config_revision: row_for_pin.config_revision,
                    agent_skills_revision: revision,
                    created_at: Utc::now(),
                };
                db.collection::<AssistantAgentLearningSkillRoot>(ROOTS_COLLECTION_NAME)
                    .insert_one(root)
                    .session(&mut *session)
                    .await?;
                Ok(revision)
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    audit(&state.db, actor, row, "assistant_learning_skill_pinned").await;
    Ok(json!({"status":"pinned","skill_id":id,"version":p.version,"skills_revision":revision}))
}

#[cfg(test)]
#[path = "assistant_agent_learning_review_tests.rs"]
mod tests;
