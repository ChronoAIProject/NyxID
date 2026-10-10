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
use futures::{StreamExt, TryStreamExt};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub const TOOL: &str = "nyxid__approve_agent_learning";
pub(crate) use super::assistant_learning_publication::FailureCode as PublicationFailureCode;
use super::assistant_learning_publication::PublicationStage;
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
        .is_some_and(|publication| publication.verified_at.is_some())
    {
        "published_unpinned"
    } else {
        "publication_failed"
    }
}

const MIGRATION_ID: &str = "publication-targets-v1";

async fn publication_ready(db: &Database) -> AppResult<()> {
    if db
        .collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME)
        .find_one(doc! {"_id":MIGRATION_ID})
        .await?
        .is_none()
    {
        return Err(AppError::Conflict(
            "nyxid_refused: publication temporarily unavailable".into(),
        ));
    }
    Ok(())
}

fn target_id(p: &LearningPublication) -> Option<String> {
    if p.target_kind.as_deref() == Some("update") {
        Some(format!("{}:{}", p.target_skill_id.as_deref()?, p.version))
    } else {
        None
    }
}

/// A publication request may have reached the registry, or its exact version
/// was verified. Such an operation is reconciled, never rewritten or released.
fn dispatched(p: &LearningPublication) -> bool {
    p.started || p.uncertain_dispatch || p.verified_at.is_some()
}

/// Card/panel code for an update package prepared before interface snapshots.
pub(crate) const LEGACY_PACKAGE: &str = "legacy_package_requires_reprepare";
const PACKAGE_FORMAT_SNAPSHOT: i64 = 2;

/// An update package prepared before #1828: no interface snapshot, so it
/// declares `category: plain` and would drop the base's tools and runtimes.
/// It is never dispatched; only reconciliation or a rebuild remains. An
/// unclassified row (`target_kind` absent) is an update iff its draft has a
/// base, which callers pass when they decoded it.
fn legacy_package(p: &LearningPublication, draft_is_update: Option<bool>) -> bool {
    p.package_format < PACKAGE_FORMAT_SNAPSHOT
        && p.interface_encrypted.is_none()
        && match p.target_kind.as_deref() {
            Some(kind) => kind == "update",
            None => draft_is_update.unwrap_or(false),
        }
}

/// Mongo form of the snapshot-less package fields. Pre-#1828 rows have no
/// `package_format` or `interface_encrypted` at all, and absent must match.
fn legacy_package_filter() -> bson::Document {
    doc! {"publication.package_format":{"$not":{"$gte":PACKAGE_FORMAT_SNAPSHOT}},
    "publication.interface_encrypted":bson::Bson::Null}
}

pub(super) fn non_effective(p: &LearningPublication, now: chrono::DateTime<Utc>) -> bool {
    !dispatched(p) && p.lease_expires_at.is_none_or(|expiry| expiry <= now)
}

/// Statuses an operation that never dispatched may hold. Pre-#1828 servers
/// recorded pre-dispatch failures as `published_unpinned` and left crashed
/// attempts `publishing`; without a dispatch, live lease or verified skill
/// those are as undispatched as `pending` and `publication_failed`.
const UNDISPATCHED_STATUSES: [&str; 4] = [
    "pending",
    "publication_failed",
    "publishing",
    "published_unpinned",
];

/// The row-level half of the undispatched fence; callers also require
/// `non_effective` on its publication.
fn undispatched_status(row: &AssistantAgentLearningProposal) -> bool {
    UNDISPATCHED_STATUSES.contains(&row.status.as_str())
        && row
            .publication
            .as_ref()
            .is_none_or(|p| p.skill_id.is_none())
}

/// Mongo form of `undispatched_status` (absent `skill_id` matches null).
fn undispatched_status_filter() -> bson::Document {
    doc! {"status":{"$in":UNDISPATCHED_STATUSES.to_vec()},"publication.skill_id":bson::Bson::Null}
}

fn review_required_failure(row: &AssistantAgentLearningProposal) -> bool {
    match row
        .failure_code
        .as_deref()
        .and_then(PublicationFailureCode::parse)
    {
        Some(PublicationFailureCode::VersionConflict) => {
            !row.publication.as_ref().is_some_and(dispatched)
        }
        Some(code) => code.requires_new_package(),
        None => false,
    }
}

/// A verified version NyxID refused to attach (changed source skill or
/// withdrawn learned evidence) is settled: checking again would only repeat
/// the refusal, so no confirmation may claim it.
fn attach_refused(row: &AssistantAgentLearningProposal) -> bool {
    row.publication
        .as_ref()
        .is_some_and(|p| p.verified_at.is_some())
        && matches!(
            row.failure_code
                .as_deref()
                .and_then(PublicationFailureCode::parse),
            Some(PublicationFailureCode::BaseChanged | PublicationFailureCode::EvidenceUnavailable)
        )
}

fn attach_settled() -> AppError {
    AppError::Conflict(
        "NyxID verified this version but did not attach it; it stays private in Ornn".into(),
    )
}

/// Codes a later confirmation must never replace: the package has to change.
fn package_refusal_codes() -> Vec<&'static str> {
    let mut codes = PublicationFailureCode::codes(PublicationFailureCode::requires_new_package);
    codes.push(PublicationFailureCode::VersionConflict.as_str());
    codes
}

async fn interface_snapshot(
    state: &AppState,
    p: &LearningPublication,
) -> AppResult<Option<publication::InterfaceSnapshot>> {
    let Some(encrypted) = &p.interface_encrypted else {
        return Ok(None);
    };
    let bytes = state
        .encryption_keys
        .decrypt(encrypted)
        .await
        .map_err(|_| conflict())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| conflict())
}

/// The target recorded in an operation's own encrypted draft: `Ok(Some(None))`
/// for a create, `Ok(Some(Some(id)))` for an update, `Ok(None)` when there is
/// no draft or it does not decode. `Err` means decryption failed now (for
/// example a key-service outage) and the migration should retry.
async fn original_target_from_body(
    state: &AppState,
    row: &AssistantAgentLearningProposal,
) -> Result<Option<Option<String>>, ()> {
    if row.body_encrypted.is_empty() {
        return Ok(None);
    }
    let bytes = state
        .encryption_keys
        .decrypt(&row.body_encrypted)
        .await
        .map_err(|_| ())?;
    let decoded = || {
        let body: Value = serde_json::from_slice(&bytes).ok()?;
        match (body.get("kind")?.as_str()?, body.get("base_skill")) {
            ("new", None | Some(Value::Null)) => Some(None),
            ("improve", Some(base)) => Some(Some(base.get("skill_id")?.as_str()?.to_owned())),
            _ => None,
        }
    };
    Ok(decoded())
}

/// Result of one pass of the publication-target migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// The completion marker exists; publication is admitted.
    Complete,
    /// An operation's target cannot be recovered from operation-bound
    /// evidence. The marker is withheld until an operator records it.
    Unresolved,
    /// A draft could not be decrypted now; another pass may classify it.
    Retry,
}

/// A draft still undecryptable on this pass is also reported unresolved, so a
/// permanently broken draft reaches operators while retries continue.
const REPORT_UNDECRYPTABLE_ON_PASS: u32 = 3;

/// Runs the migration before serving and, while a pass fails for a
/// retryable reason (storage or decryption), keeps retrying in the
/// background with capped backoff. Publication stays refused meanwhile.
pub async fn start_publication_migration(state: &AppState) {
    if publication_migration_settled(state, 1).await {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let mut delay = std::time::Duration::from_secs(5);
        let mut pass = 1;
        loop {
            tokio::time::sleep(delay).await;
            pass += 1;
            if publication_migration_settled(&state, pass).await {
                break;
            }
            delay = (delay * 2).min(std::time::Duration::from_secs(300));
        }
    });
}

async fn publication_migration_settled(state: &AppState, pass: u32) -> bool {
    match migrate_publication_targets(state, pass == REPORT_UNDECRYPTABLE_ON_PASS).await {
        Ok(MigrationOutcome::Complete) => true,
        Ok(MigrationOutcome::Unresolved) => {
            tracing::error!(
                "Skill publication migration has unresolved operations; publication remains unavailable until an operator records their targets"
            );
            true
        }
        Ok(MigrationOutcome::Retry) => {
            tracing::warn!(
                pass,
                "Skill publication migration will retry undecryptable drafts"
            );
            false
        }
        Err(error) => {
            tracing::error!(%error, pass, "Skill publication migration failed; retrying");
            false
        }
    }
}

/// Runs before publication traffic is admitted. A missing marker fails closed.
async fn report_unresolved_publication(
    state: &AppState,
    row: &AssistantAgentLearningProposal,
    operation_id: &str,
    reason: &'static str,
) {
    tracing::error!(operation_id, agent_id = %row.agent_id, proposal_id = %row.id, reason,
        "Legacy skill publication blocks publication migration");
    let _ = super::audit_service::log_system_event(
        state.db.clone(),
        "assistant_learning_migration_unresolved",
        Some(json!({"operation_id":operation_id,"agent_id":row.agent_id,"proposal_id":row.id,"reason":reason})),
    )
    .await;
}

/// One migration pass. `report_undecryptable` also reports drafts that still
/// cannot be decrypted as unresolved (see `REPORT_UNDECRYPTABLE_ON_PASS`).
pub async fn migrate_publication_targets(
    state: &AppState,
    report_undecryptable: bool,
) -> AppResult<MigrationOutcome> {
    backfill_package_format(&state.db).await?;
    let outcome = migrate_targets(state, report_undecryptable).await?;
    // After classification every dispatched legacy update has its kind.
    stamp_legacy_dispatches(&state.db).await?;
    Ok(outcome)
}

const PACKAGE_FORMAT_MIGRATION_ID: &str = "publication-package-format-v1";

/// Once: a package with a stored snapshot is format 2 (rows written by #1828
/// before `package_format` existed). Such a row is never legacy.
async fn backfill_package_format(db: &Database) -> AppResult<()> {
    let migrations = db.collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME);
    if migrations
        .find_one(doc! {"_id":PACKAGE_FORMAT_MIGRATION_ID})
        .await?
        .is_some()
    {
        return Ok(());
    }
    db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_many(
            doc! {"publication.interface_encrypted":{"$type":"binData"},
            "publication.package_format":{"$not":{"$gte":PACKAGE_FORMAT_SNAPSHOT}}},
            doc! {"$set":{"publication.package_format":PACKAGE_FORMAT_SNAPSHOT}},
        )
        .await?;
    migrations
        .update_one(
            doc! {"_id":PACKAGE_FORMAT_MIGRATION_ID},
            doc! {"$setOnInsert":{"completed_at":bson::DateTime::now()}},
        )
        .upsert(true)
        .await?;
    Ok(())
}

/// Every start: give each dispatched, unverified legacy update a set-once
/// `legacy_classified_at`. Stale release measures age from it, so nothing is
/// releasable until `min_age_hours` after this code first saw the row.
/// Bounded by the `{publication.started, status}` index.
async fn stamp_legacy_dispatches(db: &Database) -> AppResult<()> {
    let mut filter = legacy_package_filter();
    filter.extend(doc! {"publication.started":true,"status":{"$ne":"pinned"},
    "publication.target_kind":"update","publication.verified_at":bson::Bson::Null,
    "publication.legacy_classified_at":bson::Bson::Null});
    db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_many(
            filter,
            doc! {"$set":{"publication.legacy_classified_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

async fn migrate_targets(
    state: &AppState,
    report_undecryptable: bool,
) -> AppResult<MigrationOutcome> {
    if publication_ready(&state.db).await.is_ok() {
        return Ok(MigrationOutcome::Complete);
    }
    let mut rows = state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find(doc! {"publication.started":true,"status":{"$ne":"pinned"}})
        .await?;
    let mut unresolved_count = 0_usize;
    let mut retry = false;
    while let Some(row) = rows.try_next().await? {
        let Some(p) = row.publication.as_ref() else {
            unresolved_count += 1;
            report_unresolved_publication(state, &row, &row.id, "unclassified").await;
            continue;
        };
        let Ok(original_target) = original_target_from_body(state, &row).await else {
            retry = true;
            tracing::warn!(operation_id = %p.operation_id, proposal_id = %row.id,
                "Legacy skill publication draft could not be decrypted; migration will retry");
            if report_undecryptable {
                report_unresolved_publication(state, &row, &p.operation_id, "undecryptable").await;
            }
            continue;
        };
        let base_id = original_target.clone().flatten();
        if original_target.as_ref().is_some_and(|base| {
            p.target_kind
                .as_deref()
                .is_some_and(|kind| kind != if base.is_some() { "update" } else { "create" })
        }) {
            unresolved_count += 1;
            report_unresolved_publication(state, &row, &p.operation_id, "unclassified").await;
            continue;
        }
        let kind = p
            .target_kind
            .as_deref()
            .or_else(|| {
                original_target.as_ref().map(
                    |base| {
                        if base.is_some() { "update" } else { "create" }
                    },
                )
            })
            .or_else(|| p.target_skill_id.as_ref().map(|_| "update"));
        let target = p.target_skill_id.clone().or(base_id).or_else(|| {
            (kind == Some("update"))
                .then(|| p.skill_id.clone())
                .flatten()
        });
        if kind == Some("update")
            && [
                p.target_skill_id.as_ref(),
                original_target.as_ref().and_then(Option::as_ref),
                p.skill_id.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|id| Some(id.as_str()) != target.as_deref())
        {
            unresolved_count += 1;
            report_unresolved_publication(state, &row, &p.operation_id, "unclassified").await;
            continue;
        }
        let Some(kind) = kind else {
            unresolved_count += 1;
            report_unresolved_publication(state, &row, &p.operation_id, "unclassified").await;
            continue;
        };
        if kind == "update"
            && target
                .as_deref()
                .is_none_or(|id| Uuid::parse_str(id).is_err())
        {
            unresolved_count += 1;
            report_unresolved_publication(state, &row, &p.operation_id, "unclassified").await;
            continue;
        }
        if kind != "update" && kind != "create" {
            unresolved_count += 1;
            report_unresolved_publication(state, &row, &p.operation_id, "unclassified").await;
            continue;
        }
        if kind == "update" {
            let id = format!("{}:{}", target.as_deref().unwrap_or_default(), p.version);
            let existing = state
                .db
                .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
                .find_one(doc! {"_id":&id})
                .await?;
            if existing.is_none() {
                let now = Utc::now();
                let row_target = LearningPublicationTarget {
                    id,
                    agent_id: row.agent_id.clone(),
                    owner_id: row.owner_id.clone(),
                    proposal_id: row.id.clone(),
                    operation_id: p.operation_id.clone(),
                    package_sha256: p.sha256.clone(),
                    state: "uncertain".into(),
                    created_at: now,
                    updated_at: now,
                };
                if let Err(error) = state
                    .db
                    .collection(PUBLICATION_TARGETS_COLLECTION_NAME)
                    .insert_one(row_target)
                    .await
                    && !matches!(error.kind.as_ref(),
                        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
                    && !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 11000)
                {
                    return Err(error.into());
                }
            }
        }
        state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(
            doc! {"_id":&row.id,"revision":revision_filter(row.revision),"publication.operation_id":&p.operation_id,"publication.started":true},
            doc! {"$set":{"publication.target_kind":kind,"publication.target_skill_id":target,"publication.uncertain_dispatch":true}},
        ).await?;
    }
    if retry {
        return Ok(MigrationOutcome::Retry);
    }
    if unresolved_count > 0 {
        return Ok(MigrationOutcome::Unresolved);
    }
    state
        .db
        .collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":MIGRATION_ID},
            doc! {"$setOnInsert":{"completed_at":bson::DateTime::now()} },
        )
        .upsert(true)
        .await?;
    Ok(MigrationOutcome::Complete)
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
    pub source: ProposalSource,
    pub updated_at: chrono::DateTime<Utc>,
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
    /// False when learned evidence or consent was withdrawn after a request
    /// may have reached the registry: NyxID only checks it and never attaches.
    pub evidence_available: bool,
    /// Authored proposals are confirmed on their conversation card; this is the
    /// conversation of the newest one, so the panel can link to it.
    pub card_conversation_id: Option<String>,
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
    if row.source == ProposalSource::Authored {
        return Ok(agent.destroyed_at.is_none() && agent.user_id == row.owner_id);
    }
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
    if current(db, agent, row).await? {
        return Ok(());
    }
    Err(invalidate_stale(db, row).await)
}

/// Withdrawn learned evidence or consent cannot recall a request that may
/// already have reached the registry. Such an operation may still observe
/// whether its exact version landed (`Ok(false)`), but only current evidence
/// attaches it (`Ok(true)`). Any other stale draft is invalidated.
async fn evidence_allows_pin(
    db: &Database,
    agent: &AssistantAgent,
    row: &AssistantAgentLearningProposal,
) -> AppResult<bool> {
    if current(db, agent, row).await? {
        return Ok(true);
    }
    if row.publication.as_ref().is_some_and(dispatched) {
        return Ok(false);
    }
    Err(invalidate_stale(db, row).await)
}

fn must_reconcile() -> AppError {
    AppError::Conflict("Publication must be reconciled before the draft can change".into())
}

/// Invalidates a stale draft whose operation is conclusively non-effective and
/// returns the refusal to report; an effective operation is left intact.
async fn invalidate_stale(db: &Database, row: &AssistantAgentLearningProposal) -> AppError {
    if row
        .publication
        .as_ref()
        .is_some_and(|p| !non_effective(p, Utc::now()))
    {
        return must_reconcile();
    }
    let transaction_db = db.clone();
    let stale = row.clone();
    let mut session = match db.client().start_session().await {
        Ok(session) => session,
        Err(error) => return error.into(),
    };
    let invalidated = session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let fresh = transaction_db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                .find_one(doc! {"_id":&stale.id,"revision":revision_filter(stale.revision),"status":{"$nin":["pinned","rejected"]}})
                .session(&mut *session).await?.ok_or_else(conflict)?;
            if fresh.publication.as_ref().is_some_and(|p| !non_effective(p, Utc::now())) { return Err(conflict()); }
            if let Some(p) = &fresh.publication && let Some(target) = target_id(p) {
                transaction_db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME).delete_one(
                    doc! {"_id":target,"operation_id":&p.operation_id,"package_sha256":&p.sha256,"state":"reserved"}
                ).session(&mut *session).await?;
            }
            let mut filter = operation_filter(&fresh);
            filter.insert("status", doc! {"$nin":["pinned","rejected"]});
            let changed = transaction_db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(
                filter,
                doc! {"$set":{"status":"invalidated","failure_code":"evidence_unavailable","body_bytes":0,"updated_at":bson::DateTime::now()},"$unset":{"body_encrypted":""}},
            ).session(&mut *session).await?;
            if changed.modified_count != 1 { return Err(conflict()); }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await;
    match invalidated {
        Ok(()) => AppError::Conflict("Learning evidence or consent is no longer available".into()),
        Err(error) => transactions::map_transaction_error(error),
    }
}
fn revision_filter(revision: i64) -> bson::Bson {
    if revision == 0 {
        bson::to_bson(&doc! {"$in":[bson::Bson::Null,bson::Bson::Int64(0)]}).expect("static BSON")
    } else {
        revision.into()
    }
}

/// Rows written before #1828 have no `publication.attempt`; it reads as 0.
fn attempt_filter(attempt: i64) -> bson::Bson {
    revision_filter(attempt)
}

fn operation_filter(row: &AssistantAgentLearningProposal) -> bson::Document {
    let mut filter = doc! {"_id":&row.id,"revision":revision_filter(row.revision)};
    if let Some(p) = &row.publication {
        filter.insert("publication.operation_id", &p.operation_id);
    } else {
        filter.insert("publication", bson::Bson::Null);
    }
    filter
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
    if row.source == ProposalSource::Authored {
        return super::assistant_skill_authoring::decode_body(text);
    }
    let valid = learning::validate_generated(text)?.ok_or_else(not_found)?;
    serde_json::from_slice(&valid).map_err(|_| not_found())
}

/// Improvements may only target an active L1 root at its exact current B2 pin.
/// Called after model validation, on edit, and before every publication/pin.
#[cfg(test)]
pub(crate) async fn validate_base(
    db: &Database,
    agent: &AssistantAgent,
    draft: &GeneratedProposal,
) -> AppResult<()> {
    validate_proposal_base(db, agent, draft, ProposalSource::Learned).await
}

pub(crate) async fn validate_proposal_base(
    db: &Database,
    agent: &AssistantAgent,
    draft: &GeneratedProposal,
    source: ProposalSource,
) -> AppResult<()> {
    let Some(base) = &draft.base_skill else {
        return Ok(());
    };
    if !base_current(db, agent, draft, source).await? {
        return Err(AppError::Conflict("base_skill_changed".into()));
    }
    publication::next_version(Some(base))?;
    Ok(())
}

async fn base_current(
    db: &Database,
    agent: &AssistantAgent,
    draft: &GeneratedProposal,
    source: ProposalSource,
) -> AppResult<bool> {
    let Some(base) = &draft.base_skill else {
        return Ok(true);
    };
    let pin = agent.skills.iter().find(|s| {
        s.source == base.source
            && s.skill_id == base.skill_id
            && s.version == base.version
            && s.sha256 == base.sha256
            && s.name == base.name
            && s.dependencies.is_empty()
    });
    if base.source != "ornn" || pin.is_none() {
        return Ok(false);
    }
    if source == ProposalSource::Learned {
        return Ok(db.collection::<AssistantAgentLearningSkillRoot>(ROOTS_COLLECTION_NAME)
            .find_one(doc! {"agent_id":&agent.id,"owner_id":&agent.user_id,"skill_id":&base.skill_id,"version":&base.version,"sha256":&base.sha256})
            .await?.is_some());
    }
    Ok(true)
}

/// What the decoded draft says about rebuilding a legacy package.
#[derive(Clone, Copy, Default)]
struct Recovery {
    legacy: bool,
    /// The draft's base is still the agent's attached pin.
    base_current: bool,
}

async fn recovery(
    db: &Database,
    agent: &AssistantAgent,
    row: &AssistantAgentLearningProposal,
    body: Option<&GeneratedProposal>,
) -> AppResult<Recovery> {
    let Some(p) = row.publication.as_ref() else {
        return Ok(Recovery::default());
    };
    let legacy = legacy_package(p, body.map(|b| b.base_skill.is_some()));
    let base_current = match body {
        Some(body) if legacy => base_current(db, agent, body, row.source).await?,
        _ => false,
    };
    Ok(Recovery {
        legacy,
        base_current,
    })
}

/// Recorded by a rebuild that found the base cannot be reproduced; another
/// rebuild cannot cure them, only a revised draft.
const NOT_REBUILDABLE: [PublicationFailureCode; 3] = [
    PublicationFailureCode::BaseChanged,
    PublicationFailureCode::BaseVerifyFailed,
    PublicationFailureCode::BaseInterfaceIncompatible,
];

fn reprepare_offered(
    row: &AssistantAgentLearningProposal,
    recovery: Recovery,
    now: chrono::DateTime<Utc>,
) -> bool {
    recovery.legacy
        && recovery.base_current
        && row
            .publication
            .as_ref()
            .is_some_and(|p| non_effective(p, now))
        && undispatched_status(row)
        && !row
            .failure_code
            .as_deref()
            .and_then(PublicationFailureCode::parse)
            .is_some_and(|code| NOT_REBUILDABLE.contains(&code))
}

/// Pre-#1828 servers stored this for every failed attempt, whatever its stage.
const LEGACY_RETRY_CODE: &str = "publication_retry_required";

/// A legacy package that was never dispatched shows the rebuild prompt, or
/// the source change when its base moved on. A dispatched attempt that a
/// pre-#1828 server only marked for retry shows that its outcome is unknown.
/// Anything else keeps its code.
fn shown_failure_code(
    row: &AssistantAgentLearningProposal,
    recovery: Recovery,
    now: chrono::DateTime<Utc>,
) -> Option<&str> {
    let undispatched_legacy = recovery.legacy
        && row
            .publication
            .as_ref()
            .is_some_and(|p| non_effective(p, now));
    if reprepare_offered(row, recovery, now) {
        Some(LEGACY_PACKAGE)
    } else if undispatched_legacy && !recovery.base_current {
        Some(PublicationFailureCode::BaseChanged.as_str())
    } else if row.failure_code.as_deref() == Some(LEGACY_RETRY_CODE)
        && row
            .publication
            .as_ref()
            .is_some_and(|p| dispatched(p) && p.verified_at.is_none())
    {
        Some(PublicationFailureCode::PublishUncertain.as_str())
    } else {
        row.failure_code.as_deref()
    }
}

/// Pre-#1828 servers stored a pre-dispatch failure as `published_unpinned`;
/// without a verified version it is shown as the failure it was.
fn shown_status(row: &AssistantAgentLearningProposal) -> &str {
    let verified = row
        .publication
        .as_ref()
        .is_some_and(|p| p.verified_at.is_some() || p.skill_id.is_some());
    if row.status == "published_unpinned" && !verified {
        "publication_failed"
    } else {
        &row.status
    }
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
        let evidence_available = row.status == "pinned" || current(&state.db, &agent, &row).await?;
        let valid = evidence_available
            || row
                .publication
                .as_ref()
                .is_some_and(|p| !non_effective(p, Utc::now()));
        if !valid {
            let _ = invalidate_stale(&state.db, &row).await;
            continue;
        }
        let card_conversation_id = if row.source == ProposalSource::Authored {
            state
                .db
                .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                    crate::models::assistant_acknowledgement::COLLECTION_NAME,
                )
                .find_one(
                    doc! {"user_id":actor,"tool_name":TOOL,"authored_skill.proposal_id":&row.id},
                )
                .sort(doc! {"created_at":-1})
                .await?
                .map(|card| card.conversation_id)
        } else {
            None
        };
        let body = if include_drafts && row.status != "pinned" {
            Some(draft(state, &row).await?)
        } else {
            None
        };
        let failure_code = match body.as_ref() {
            Some(draft) => shown_failure_code(
                &row,
                recovery(&state.db, &agent, &row, Some(draft)).await?,
                Utc::now(),
            )
            .map(str::to_owned),
            // Without the draft only the dispatch-state mappings apply.
            None => shown_failure_code(&row, Recovery::default(), Utc::now()).map(str::to_owned),
        };
        let status = shown_status(&row).to_owned();
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
            source: row.source,
            updated_at: row.updated_at,
            id: row.id,
            agent_id: row.agent_id,
            owner_id: row.owner_id,
            run_id: row.run_id,
            status,
            revision: row.revision,
            config_revision: row.config_revision,
            agent_skills_revision: row.agent_skills_revision,
            current_skills_revision: agent.skills_revision,
            model_contract: row.model_contract,
            evidence_count: row.evidence.len(),
            body_bytes: row.body_bytes,
            created_at: row.created_at,
            failure_code,
            evidence,
            draft: body,
            published_skill_id: row.publication.as_ref().and_then(|p| p.skill_id.clone()),
            published_version: row.publication.as_ref().map(|p| p.version.clone()),
            evidence_available,
            card_conversation_id,
        });
    }
    Ok(result)
}

async fn audit(db: &Database, actor: &str, row: &AssistantAgentLearningProposal, event: &str) {
    let _ = super::audit_service::log_actor_event(db.clone(), &super::audit_service::AuditActor {user_id:actor.into(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},event,
        Some(json!({"agent_id":row.agent_id,"owner_id":row.owner_id,"proposal_id":row.id,"revision":row.revision,"evidence_count":row.evidence.len()}))).await;
}

struct DeferredAudit<'a> {
    code: &'a str,
    stage: &'a str,
    status: Option<i32>,
    /// The card the owner clicked, when it is not the approving card.
    clicked: Option<&'a str>,
}

async fn audit_deferred(
    db: &Database,
    actor: &str,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    event: DeferredAudit<'_>,
) {
    let _ = super::audit_service::log_actor_event(db.clone(), &super::audit_service::AuditActor {user_id:actor.into(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},"assistant_learning_publication_deferred",
        Some(json!({"agent_id":row.agent_id,"proposal_id":row.id,"revision":row.revision,"failure_code":event.code,"stage":event.stage,"attempt":p.attempt,"registry_status":event.status,"operation_id":p.operation_id,"acknowledgement_id":p.acknowledgement_id,"clicked_acknowledgement_id":event.clicked}))).await;
}

struct DeferredFailure {
    code: publication::FailureCode,
    stage: PublicationStage,
    status: Option<i32>,
    definitive: bool,
}

async fn defer(
    state: &AppState,
    actor: &str,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    failure: DeferredFailure,
) -> AppResult<()> {
    let DeferredFailure {
        code,
        stage,
        status,
        definitive,
    } = failure;
    let db = state.db.clone();
    let filter = lease_filter(row, p);
    let target = target_id(p);
    let operation = p.operation_id.clone();
    let sha = p.sha256.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            // Status and release follow the fenced durable row (for example a
            // verified checkpoint written by this attempt), not the claim snapshot.
            let fresh = db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                .find_one(filter.clone()).session(&mut *session).await?.ok_or_else(conflict)?;
            let mut after = fresh.publication.clone().ok_or_else(conflict)?;
            after.started = false;
            after.lease_expires_at = None;
            let releasable = definitive && non_effective(&after, Utc::now());
            let mut fields = doc! {"status":failure_status(&fresh),"failure_code":code.as_str(),"publication.last_stage":stage.as_str(),"publication.last_registry_status":status,"publication.lease_expires_at":bson::Bson::Null,"updated_at":bson::DateTime::now()};
            if releasable {
                fields.insert("publication.started", false);
            } else if matches!(stage, PublicationStage::Publish | PublicationStage::Reconcile) {
                fields.insert("publication.uncertain_dispatch", true);
            }
            let changed = db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                .update_one(filter.clone(), doc! {"$set":fields}).session(&mut *session).await?;
            if changed.modified_count != 1 { return Err(conflict()); }
            if let Some(target) = target.as_ref() {
                if releasable {
                    db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME)
                        .delete_one(doc! {"_id":target,"operation_id":&operation,"package_sha256":&sha,"state":{"$in":["reserved","uncertain"]}})
                        .session(&mut *session).await?;
                } else {
                    // Never downgrade a landed (verified) target.
                    db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME)
                        .update_one(doc! {"_id":target,"operation_id":&operation,"package_sha256":&sha,"state":"reserved"},
                            doc! {"$set":{"state":"uncertain","updated_at":bson::DateTime::now()}})
                        .session(&mut *session).await?;
                }
            }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
    audit_deferred(
        &state.db,
        actor,
        row,
        p,
        DeferredAudit {
            code: code.as_str(),
            stage: stage.as_str(),
            status,
            clicked: None,
        },
    )
    .await;
    Ok(())
}

async fn defer_publication_error<T>(
    state: &AppState,
    actor: &str,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    stage: PublicationStage,
    error: publication::PublicationError,
    fallback_status: Option<u16>,
) -> AppResult<T> {
    use publication::FailureCode as Code;
    let code = error.failure_code(stage);
    let status = error.status().or(fallback_status).map(i32::from);
    defer(
        state,
        actor,
        row,
        p,
        DeferredFailure {
            code,
            stage,
            status,
            definitive: !p.started
                && !p.uncertain_dispatch
                && matches!(
                    stage,
                    PublicationStage::FormatValidate | PublicationStage::BaseVerify
                ),
        },
    )
    .await?;
    match code {
        Code::VersionConflict => Err(AppError::Conflict("version_conflict".into())),
        Code::PublishUncertain => Err(publication::ambiguous()),
        _ => Err(error.into_app_error()),
    }
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
    if row.revision != expected_revision
        || !matches!(row.status.as_str(), "pending" | "publication_failed")
    {
        return Err(conflict());
    }
    if row
        .publication
        .as_ref()
        .is_some_and(|p| !non_effective(p, Utc::now()))
    {
        return Err(must_reconcile());
    }
    let encoded = if row.source == ProposalSource::Authored {
        let body = super::assistant_skill_authoring::decode_body(&value.to_string())?;
        super::assistant_skill_authoring::validate_body(&body)?
    } else {
        learning::validate_generated(&value.to_string())?.ok_or_else(conflict)?
    };
    let value: GeneratedProposal = serde_json::from_slice(&encoded).map_err(|_| conflict())?;
    validate_proposal_base(&state.db, &agent, &value, row.source).await?;
    let encrypted = state.encryption_keys.encrypt(&encoded).await?;
    let fingerprint = super::assistant_action_receipts::fingerprint_sensitive_material(&format!(
        "{}:{}:{}",
        agent.id,
        row.model_contract,
        String::from_utf8_lossy(&encoded)
    ));
    let db = state.db.clone();
    let proposal_id = id.to_owned();
    let skills_revision = agent.skills_revision;
    let body_len = encoded.len() as i64;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let fresh = db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                .find_one(doc! {"_id":&proposal_id,"revision":revision_filter(expected_revision),"status":{"$in":["pending","publication_failed"]}})
                .session(&mut *session).await?.ok_or_else(conflict)?;
            if fresh.publication.as_ref().is_some_and(|p| !non_effective(p, Utc::now())) { return Err(must_reconcile()); }
            if let Some(p) = &fresh.publication && let Some(target) = target_id(p) {
                db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME).delete_one(
                    doc! {"_id":target,"operation_id":&p.operation_id,"package_sha256":&p.sha256,"state":"reserved"}
                ).session(&mut *session).await?;
            }
            let mut filter = operation_filter(&fresh);
            filter.insert("status", doc! {"$in":["pending","publication_failed"]});
            let changed = db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(
                filter,
                doc! {"$set":{"status":"pending","body_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted.clone()},"body_bytes":body_len,"fingerprint":&fingerprint,"revision":expected_revision+1,"agent_skills_revision":skills_revision,"updated_at":bson::DateTime::now()},"$unset":{"publication":"","failure_code":""}},
            ).session(&mut *session).await?;
            if changed.modified_count != 1 { return Err(conflict()); }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
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
                // Includes pre-#1828 pre-dispatch failures (see UNDISPATCHED_STATUSES).
                let fresh = db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                    .find_one(doc! {"_id":&proposal_id,"revision":revision_filter(revision),"status":{"$in":UNDISPATCHED_STATUSES.to_vec()}})
                    .session(&mut *session).await?.ok_or_else(conflict)?;
                if fresh.publication.as_ref().is_some_and(|p| !non_effective(p, Utc::now())) || !undispatched_status(&fresh) { return Err(must_reconcile()); }
                if let Some(p) = &fresh.publication && let Some(target) = target_id(p) {
                    db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME).delete_one(
                        doc! {"_id":target,"operation_id":&p.operation_id,"package_sha256":&p.sha256,"state":"reserved"}
                    ).session(&mut *session).await?;
                }
                let changed = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(
                        { let mut filter = operation_filter(&fresh); filter.insert("status", doc! {"$in":UNDISPATCHED_STATUSES.to_vec()}); filter },
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

#[cfg(test)]
pub(super) struct UnavailableReader;
#[cfg(test)]
#[async_trait::async_trait]
impl OrnnReader for UnavailableReader {
    async fn get(&self, _path: &str) -> AppResult<Vec<u8>> {
        Err(conflict())
    }
}

pub(crate) struct BindingFailure {
    pub(crate) code: Option<publication::FailureCode>,
    pub(crate) error: AppError,
}

impl From<AppError> for BindingFailure {
    fn from(error: AppError) -> Self {
        Self { code: None, error }
    }
}

impl From<mongodb::error::Error> for BindingFailure {
    fn from(error: mongodb::error::Error) -> Self {
        Self::from(AppError::from(error))
    }
}

impl BindingFailure {
    /// The binding snapshot reads and verifies the base before any card exists.
    fn from_publication(error: publication::PublicationError) -> Self {
        Self {
            code: Some(error.failure_code(PublicationStage::BaseVerify)),
            error: error.into_app_error(),
        }
    }
}

pub async fn approval_binding<R: OrnnReader>(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    id: &str,
    expected_revision: i64,
    expected_skills_revision: i64,
    reader: &R,
) -> AppResult<Value> {
    approval_binding_typed(
        state,
        actor,
        agent_id,
        id,
        expected_revision,
        expected_skills_revision,
        reader,
    )
    .await
    .map_err(|failure| failure.error)
}

pub(crate) async fn approval_binding_typed<R: OrnnReader>(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    id: &str,
    expected_revision: i64,
    expected_skills_revision: i64,
    reader: &R,
) -> Result<Value, BindingFailure> {
    publication_ready(&state.db).await?;
    let (agent, mut row) = load(&state.db, actor, agent_id, id).await?;
    if agent.user_id != actor {
        return Err(AppError::Conflict("owner_binding_unavailable".into()).into());
    }
    if attach_refused(&row) {
        return Err(attach_settled().into());
    }
    if row.revision != expected_revision
        || agent.skills_revision != expected_skills_revision
        || review_required_failure(&row)
        || !matches!(
            row.status.as_str(),
            "pending" | "publication_failed" | "published_unpinned" | "publishing"
        )
    {
        return Err(conflict().into());
    }
    evidence_allows_pin(&state.db, &agent, &row).await?;
    let body = draft(state, &row).await?;
    // A possibly dispatched operation may still be checked after its base
    // changed; the pin gate refuses to attach the result.
    if !row.publication.as_ref().is_some_and(dispatched)
        && !base_current(&state.db, &agent, &body, row.source).await?
    {
        record_base_changed_if_current(&state.db, &row).await?;
        return Err(BindingFailure {
            code: Some(publication::FailureCode::BaseChanged),
            error: AppError::Conflict("base_skill_changed".into()),
        });
    }
    if row.publication.is_none() {
        let operation = Uuid::new_v4().to_string();
        let name = body.base_skill.as_ref().map_or_else(
            || {
                if row.source == ProposalSource::Authored {
                    body.name.clone()
                } else {
                    format!("nyx-learning-{}", operation.replace('-', ""))
                }
            },
            |b| b.name.clone(),
        );
        let version = publication::next_version(body.base_skill.as_ref())?;
        let snapshot = if let Some(base) = &body.base_skill {
            Some(
                publication::snapshot(reader, actor, base)
                    .await
                    .map_err(BindingFailure::from_publication)?,
            )
        } else {
            None
        };
        let bytes = publication::package_with_snapshot(
            &body,
            &operation,
            &name,
            &version,
            snapshot.as_ref(),
        )?;
        let encrypted = match snapshot {
            Some(value) => Some(
                state
                    .encryption_keys
                    .encrypt(&serde_json::to_vec(&value).map_err(|_| conflict())?)
                    .await?,
            ),
            None => None,
        };
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
            target_kind: Some(
                if body.base_skill.is_some() {
                    "update"
                } else {
                    "create"
                }
                .into(),
            ),
            target_skill_id: body.base_skill.as_ref().map(|base| base.skill_id.clone()),
            interface_encrypted: encrypted,
            package_format: PACKAGE_FORMAT_SNAPSHOT,
            ..Default::default()
        };
        state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(doc! {"_id":id,"revision":revision_filter(row.revision),"status":"pending","publication":bson::Bson::Null},doc! {"$set":{"publication":bson::to_bson(&p).map_err(|_| conflict())?}}).await?;
        row = load(&state.db, actor, agent_id, id).await?.1;
    }
    if row
        .publication
        .as_ref()
        .is_some_and(|p| p.target_kind.is_none())
    {
        let kind = if body.base_skill.is_some() {
            "update"
        } else {
            "create"
        };
        let target = body.base_skill.as_ref().map(|b| b.skill_id.clone());
        let p = row.publication.as_ref().ok_or_else(conflict)?;
        let changed = state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(
            doc! {"_id":id,"revision":revision_filter(row.revision),"publication.operation_id":&p.operation_id,"publication.target_kind":bson::Bson::Null},
            doc! {"$set":{"publication.target_kind":kind,"publication.target_skill_id":target}},
        ).await?;
        if changed.modified_count != 1 {
            return Err(conflict().into());
        }
        row = load(&state.db, actor, agent_id, id).await?.1;
    }
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    match (p.target_kind.as_deref(), body.base_skill.as_ref()) {
        (Some("update"), Some(base))
            if p.target_skill_id.as_deref() == Some(base.skill_id.as_str()) => {}
        (Some("create"), None) if p.target_skill_id.is_none() => {}
        _ => return Err(conflict().into()),
    }
    // No new confirmation for a package that can never be dispatched; a
    // dispatched one may still be checked.
    if legacy_package(p, None) && !dispatched(p) {
        return Err(AppError::Conflict(LEGACY_PACKAGE.into()).into());
    }
    if p.lease_expires_at.is_some_and(|t| t > Utc::now()) {
        return Err(
            AppError::Conflict("Publication is in progress; retry when it settles".into()).into(),
        );
    }
    Ok(publication::binding(&row, p, agent.skills_revision))
}

/// A changed base is definitive for an operation that never dispatched: it
/// replaces any retryable code (so the card stops offering a doomed retry)
/// but never a package refusal, a newer attempt or a live lease.
async fn record_base_changed_if_current(
    db: &Database,
    row: &AssistantAgentLearningProposal,
) -> AppResult<()> {
    let Some(observed) = row.publication.as_ref() else {
        return Ok(());
    };
    db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id,"revision":revision_filter(row.revision),"publication.operation_id":&observed.operation_id,
                "publication.attempt":attempt_filter(observed.attempt),"publication.lease_expires_at":bson::Bson::Null,
                "publication.started":{"$ne":true},"publication.uncertain_dispatch":{"$ne":true},
                "publication.verified_at":bson::Bson::Null,"status":{"$in":["pending","publication_failed"]},
                "failure_code":{"$nin":package_refusal_codes()}},
            doc! {"$set":{"failure_code":PublicationFailureCode::BaseChanged.as_str(),"status":"publication_failed",
                "publication.last_stage":"pre_claim","updated_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

/// The only card an HTTP learned confirmation may act on: the owner's own
/// learning action card for this exact binding, in the NyxBot conversation
/// that raised it. A live pending card is allowed and audited here; a used
/// card can only resume the operation it approved (the claim checks that).
/// Any other card id, including service, account and authored cards, is
/// refused without changing it.
pub async fn confirm_learned_card(
    state: &AppState,
    chat: &ChatAuthority,
    auditor: &super::audit_service::AuditActor,
    card_id: &str,
    binding: &Value,
) -> AppResult<()> {
    let card = state
        .db
        .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":card_id,"user_id":&chat.user_id,"kind":"action","tool_name":TOOL,
            "authored_skill":bson::Bson::Null,"conversation_id":&chat.conversation_id,
            "arguments_digest":acks::arguments_digest(binding),
            "$or":[{"status":{"$in":["pending","allowed"]},"expires_at":{"$gt":bson::DateTime::now()}},{"status":"used"}]})
        .await?
        .ok_or_else(|| {
            AppError::Conflict("Learning approval card is missing, expired, used or stale".into())
        })?;
    if card.status == "pending" {
        let decided = acks::decide(
            &state.db,
            &chat.user_id,
            &card.conversation_id,
            &card.id,
            true,
        )
        .await?;
        acks::audit_decision(&state.db, auditor, &decided).await;
    }
    Ok(())
}

/// A renewal re-raises the original card's exact operation: the fresh binding
/// must equal the original digest at the original card's skills revision.
pub fn require_renewal_of(
    original: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
    binding: &Value,
) -> AppResult<()> {
    let changed = || AppError::Conflict("Skill review changed".into());
    let reference = original.authored_skill.as_ref().ok_or_else(changed)?;
    let mut original_args = binding.clone();
    original_args["skills_revision"] = json!(reference.skills_revision);
    original_args["authored_skill"]["skills_revision"] = json!(reference.skills_revision);
    if original.arguments_digest.as_deref() != Some(acks::arguments_digest(&original_args).as_str())
    {
        return Err(changed());
    }
    Ok(())
}

/// The release transition shared by the single and the stale bulk release:
/// the operation becomes conclusively non-effective.
fn released_fields(status: &str) -> bson::Document {
    let mut fields = doc! {"publication.started":false,"publication.uncertain_dispatch":false,
    "publication.last_stage":"operator_released","updated_at":bson::DateTime::now()};
    if matches!(status, "pending" | "publishing" | "publication_failed") {
        fields.insert("status", "publication_failed");
        fields.insert(
            "failure_code",
            PublicationFailureCode::OperatorReleased.as_str(),
        );
    }
    fields
}

/// Admin recovery for a target reserved by an operation whose dispatch is
/// uncertain (typically a migrated legacy attempt), used only after the
/// operator holds authoritative evidence, from Ornn's version list and its own
/// request records for this operation ID, that the request had no effect and
/// can no longer have one. The operation becomes non-effective so its owner
/// can retry the same reviewed package (a legacy package is rebuilt first) or
/// discard it. NyxID never skips to another version on its own.
pub async fn operator_release_target(
    state: &AppState,
    operator: &super::audit_service::AuditActor,
    proposal_id: &str,
    operation_id: &str,
    evidence_ref: &str,
) -> AppResult<Value> {
    if evidence_ref.is_empty()
        || evidence_ref.len() > 128
        || !evidence_ref
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._:/#-".contains(&c))
    {
        return Err(AppError::ValidationError(
            "evidence_ref must be a short reference to the operator's evidence".into(),
        ));
    }
    let db = state.db.clone();
    let id = proposal_id.to_owned();
    let operation = operation_id.to_owned();
    let mut session = db.client().start_session().await?;
    let (row, target, previous_state) = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<(AssistantAgentLearningProposal, String, String)> = async {
                let row = db
                    .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                    .find_one(doc! {"_id":&id,"publication.operation_id":&operation})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                let p = row.publication.as_ref().ok_or_else(not_found)?;
                if row.status == "pinned"
                    || p.verified_at.is_some()
                    || p.lease_expires_at.is_some_and(|t| t > Utc::now())
                {
                    return Err(AppError::Conflict(
                        "Only an unverified publication without a live attempt can be released"
                            .into(),
                    ));
                }
                let target = target_id(p).ok_or_else(|| {
                    AppError::Conflict("This publication holds no update target".into())
                })?;
                let holder = db
                    .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
                    .find_one(doc! {"_id":&target,"operation_id":&operation,"state":{"$in":["reserved","uncertain"]}})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| {
                        AppError::Conflict("This publication does not hold a releasable target".into())
                    })?;
                db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME)
                    .delete_one(doc! {"_id":&target,"operation_id":&operation,"state":&holder.state})
                    .session(&mut *session)
                    .await?;
                let fields = released_fields(&row.status);
                let changed = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&id,"revision":revision_filter(row.revision),"publication.operation_id":&operation,
                            "publication.attempt":attempt_filter(p.attempt),"publication.verified_at":bson::Bson::Null},
                        doc! {"$set":fields},
                    )
                    .session(&mut *session)
                    .await?;
                if changed.modified_count != 1 {
                    return Err(conflict());
                }
                Ok((row, target, holder.state))
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    let _ = super::audit_service::log_actor_event(
        state.db.clone(),
        operator,
        "assistant_learning_publication_target_released",
        Some(json!({"agent_id":row.agent_id,"owner_id":row.owner_id,"proposal_id":row.id,
            "operation_id":operation_id,"target":target,"previous_state":previous_state,"evidence_ref":evidence_ref})),
    )
    .await;
    Ok(
        json!({"status":"released","proposal_id":row.id,"operation_id":operation_id,"target":target}),
    )
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
    publication_ready(&state.db).await?;
    let actor = &chat.user_id;
    let (agent, row) = load(&state.db, actor, agent_id, id).await?;
    if !approval_actor_allowed(&agent, actor, chat) {
        return Err(AppError::Forbidden("owner_binding_unavailable".into()));
    }
    evidence_allows_pin(&state.db, &agent, &row).await?;
    let mut p = row.publication.clone().ok_or_else(conflict)?;
    if publication::binding(&row, &p, agent.skills_revision) != *binding {
        return Err(conflict());
    }
    let digest = acks::arguments_digest(binding);
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
    let mut session = db.client().start_session().await?;
    let claim_future = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<LearningPublication> = async {
                let fresh = db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                    .find_one(doc! {"_id":&proposal_id,"revision":revision_filter(proposal_revision),"publication.operation_id":&operation_id})
                    .session(&mut *session).await?.ok_or_else(conflict)?;
                let mut next = fresh.publication.clone().ok_or_else(conflict)?;
                if publication::binding(&fresh, &next, agent_for_claim.skills_revision) != binding { return Err(conflict()); }
                // An unclassified row is refused just below.
                if legacy_package(&next, None) && !dispatched(&next) {
                    return Err(AppError::Conflict(LEGACY_PACKAGE.into()));
                }
                if review_required_failure(&fresh) { return Err(conflict()); }
                if attach_refused(&fresh) { return Err(attach_settled()); }
                let classified = match next.target_kind.as_deref() {
                    Some("create") => next.target_skill_id.is_none(),
                    Some("update") => next.target_skill_id.as_deref().is_some_and(|id| Uuid::parse_str(id).is_ok()),
                    _ => false,
                };
                if !classified || next.lease_expires_at.is_some_and(|t| t > Utc::now()) {
                    return Err(conflict());
                }
                if !resumes_publication(&next, &actor_for_claim, &card, &digest)
                    && !acks::consume_action_in_session(&db, &chat, &card, TOOL, &binding, &mut *session).await? {
                    return Err(AppError::Conflict(if fresh.source == ProposalSource::Learned {
                        "Learning approval card is missing, expired, used or stale"
                    } else {
                        "This confirmation cannot publish right now; see the card for the reason"
                    }.into()));
                }
                if let Some(target) = target_id(&next) {
                    let targets = db.collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME);
                    if let Some(holder) = targets.find_one(doc! {"_id":&target}).session(&mut *session).await? {
                        if (holder.operation_id != next.operation_id || holder.package_sha256 != next.sha256) && !next.started {
                            return Err(AppError::Conflict("target_busy: Another draft is publishing this skill version".into()));
                        }
                    } else {
                        let now = Utc::now();
                        targets.insert_one(LearningPublicationTarget { id: target, agent_id: fresh.agent_id.clone(), owner_id: fresh.owner_id.clone(), proposal_id: fresh.id.clone(), operation_id: next.operation_id.clone(), package_sha256: next.sha256.clone(), state: "reserved".into(), created_at: now, updated_at: now }).session(&mut *session).await.map_err(|error| {
                            if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000) {
                                AppError::Conflict("target_busy: Another draft is publishing this skill version".into())
                            } else { AppError::from(error) }
                        })?;
                    }
                }
                next.approved_by = Some(actor_for_claim.clone());
                next.acknowledgement_id = Some(card.clone());
                next.approval_digest = Some(digest.clone());
                next.skills_revision = agent_for_claim.skills_revision;
                next.lease_id = Some(lease.clone());
                next.lease_expires_at = Some(Utc::now() + Duration::seconds(LEASE_SECONDS));
                next.attempt += 1;
                next.last_stage = Some("claimed".into());
                next.last_registry_status = None;
                let changed = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&proposal_id,"revision":revision_filter(proposal_revision),"publication.operation_id":&operation_id,"status":{"$in":["pending","publishing","publication_failed","published_unpinned"]},"$or":[{"publication.lease_expires_at":bson::Bson::Null},{"publication.lease_expires_at":{"$lte":bson::DateTime::now()}}]},
                        doc! {"$set":{"status":"publishing","publication.approved_by":&next.approved_by,"publication.acknowledgement_id":&next.acknowledgement_id,"publication.approval_digest":&next.approval_digest,"publication.skills_revision":next.skills_revision,"publication.lease_id":&next.lease_id,"publication.lease_expires_at":next.lease_expires_at.map(bson::DateTime::from_chrono),"publication.attempt":next.attempt,"publication.last_stage":"claimed","publication.last_registry_status":bson::Bson::Null,"failure_code":bson::Bson::Null,"updated_at":bson::DateTime::now()}},
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
        });
    p = claim_future
        .await
        .map_err(transactions::map_transaction_error)?;
    audit(
        &state.db,
        &actor,
        &row,
        "assistant_learning_publication_approved",
    )
    .await;
    let result = execute(state, &actor, &row, &p, reader).await;
    if result.is_err() {
        let durable = state
            .db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .await?
            .ok_or_else(not_found)?;
        if durable.status == "publishing" || durable.status == "published_unpinned" {
            let status = failure_status(&durable).to_owned();
            let observed = durable.publication.as_ref();
            let stage = observed
                .and_then(|v| v.last_stage.as_deref())
                .unwrap_or("preflight")
                .to_owned();
            // Registry outcomes are recorded where they occur. Anything left
            // here failed inside NyxID (lost lease, changed state, storage), so
            // its code follows only from what may already have reached the
            // registry, never from the stage name.
            let code = if observed.is_some_and(|v| v.verified_at.is_some()) {
                publication::FailureCode::PinConflict
            } else if observed.is_some_and(dispatched) {
                publication::FailureCode::PublishUncertain
            } else {
                publication::FailureCode::NyxidRefused
            }
            .as_str()
            .to_owned();
            let mut after = durable.publication.clone().ok_or_else(conflict)?;
            after.lease_expires_at = None;
            let release = non_effective(&after, Utc::now());
            let target = target_id(&after);
            let db = state.db.clone();
            let proposal_id = id.to_owned();
            let operation = p.operation_id.clone();
            let sha = p.sha256.clone();
            // A failure already recorded under this lease cleared its expiry.
            let filter = doc! {"_id":id,"revision":revision_filter(row.revision),"publication.operation_id":&p.operation_id,"publication.attempt":p.attempt,"publication.lease_id":&p.lease_id,"publication.lease_expires_at":{"$ne":bson::Bson::Null},"status":{"$in":["publishing","published_unpinned"]}};
            let audit_code = code.clone();
            let audit_stage = stage.clone();
            let response_status: Option<i32> = None;
            let mut session = db.client().start_session().await?;
            let changed = session.start_transaction().and_run2(async move |session| {
                let db = db.clone();
                let filter = filter.clone();
                let target = target.clone();
                let proposal_id = proposal_id.clone();
                let operation = operation.clone();
                let sha = sha.clone();
                let code = code.clone();
                let stage = stage.clone();
                let status = status.clone();
                let result: AppResult<bool> = async {
                    let changed = db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                        .update_one(filter, doc! {"$set":{"status":status,"failure_code":code,"publication.last_stage":stage,"publication.last_registry_status":response_status,"publication.lease_expires_at":bson::Bson::Null}})
                        .session(&mut *session).await?;
                    if changed.modified_count == 1 && release && let Some(target) = target {
                        db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME)
                            .delete_one(doc! {"_id":target,"proposal_id":&proposal_id,"operation_id":operation,"package_sha256":sha,"state":"reserved"})
                            .session(&mut *session).await?;
                    }
                    Ok(changed.modified_count == 1)
                }.await;
                transactions::transaction_result(result)
            }).await.map_err(transactions::map_transaction_error)?;
            if changed {
                audit_deferred(
                    &state.db,
                    &actor,
                    &row,
                    &p,
                    DeferredAudit {
                        code: &audit_code,
                        stage: &audit_stage,
                        status: response_status,
                        clicked: None,
                    },
                )
                .await;
            }
        }
    }
    result
}
fn lease_filter(row: &AssistantAgentLearningProposal, p: &LearningPublication) -> bson::Document {
    doc! {"_id":&row.id,"revision":revision_filter(row.revision),"status":{"$in":["publishing","published_unpinned"]},"publication.operation_id":&p.operation_id,"publication.attempt":p.attempt,"publication.lease_id":&p.lease_id,"publication.lease_expires_at":{"$gt":bson::DateTime::now()}}
}
async fn set_stage(
    state: &AppState,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    stage: &str,
) -> AppResult<()> {
    let changed = state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            lease_filter(row, p),
            doc! {"$set":{"publication.last_stage":stage}},
        )
        .await?;
    if changed.modified_count != 1 {
        return Err(conflict());
    }
    Ok(())
}

async fn mark_started(
    state: &AppState,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
) -> AppResult<()> {
    let db = state.db.clone();
    let filter = lease_filter(row, p);
    let target = target_id(p);
    let operation = p.operation_id.clone();
    let sha = p.sha256.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let changed = db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                .update_one(filter.clone(), vec![doc! {"$set":{"publication.started":true,"publication.dispatched_at":{"$ifNull":["$publication.dispatched_at",bson::DateTime::now()]},"publication.last_stage":"publish"}}])
                .session(&mut *session).await?;
            if changed.modified_count != 1 { return Err(conflict()); }
            if let Some(target) = target.as_ref() {
                let changed = db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME)
                    .update_one(doc! {"_id":target,"operation_id":&operation,"package_sha256":&sha,"state":"reserved"},
                        doc! {"$set":{"state":"uncertain","updated_at":bson::DateTime::now()}})
                    .session(&mut *session).await?;
                if changed.matched_count != 1 { return Err(conflict()); }
            }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
    Ok(())
}
/// Re-validates the claimed operation against live state. Returns the agent
/// and, when the result must not be attached, why: withdrawn learned evidence
/// or a changed source skill on an operation that may already have
/// dispatched. Such an operation still reconciles and verifies read-only.
async fn recheck(
    state: &AppState,
    actor: &str,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    body: &GeneratedProposal,
    stage: PublicationStage,
) -> AppResult<(AssistantAgent, Option<PublicationFailureCode>)> {
    let (agent, latest) = load(&state.db, actor, &row.agent_id, &row.id).await?;
    let evidence_current = evidence_allows_pin(&state.db, &agent, &latest).await?;
    if latest.revision != row.revision
        || latest.fingerprint != row.fingerprint
        || agent.skills_revision != p.skills_revision
        || latest.publication.as_ref().is_none_or(|v| {
            v.lease_id != p.lease_id || v.lease_expires_at.is_none_or(|t| t <= Utc::now())
        })
    {
        return Err(conflict());
    }
    let mut withheld = (!evidence_current).then_some(PublicationFailureCode::EvidenceUnavailable);
    if !base_current(&state.db, &agent, body, row.source).await? {
        if !latest.publication.as_ref().is_some_and(dispatched) {
            defer(
                state,
                actor,
                row,
                p,
                DeferredFailure {
                    code: PublicationFailureCode::BaseChanged,
                    stage,
                    status: None,
                    definitive: true,
                },
            )
            .await?;
            return Err(AppError::Conflict("base_skill_changed".into()));
        }
        withheld = withheld.or(Some(PublicationFailureCode::BaseChanged));
    }
    publication::next_version(body.base_skill.as_ref())?;
    Ok((agent, withheld))
}
async fn execute(
    state: &AppState,
    actor: &str,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    reader: &impl OrnnReader,
) -> AppResult<Value> {
    let body = draft(state, row).await?;
    set_stage(state, row, p, PublicationStage::Preflight.as_str()).await?;
    recheck(state, actor, row, p, &body, PublicationStage::Preflight).await?;
    let id = if p.verified_at.is_some() {
        p.skill_id.clone().ok_or_else(conflict)?
    } else if p.started || p.uncertain_dispatch {
        match publication::reconcile(reader, actor, p, body.base_skill.as_ref()).await {
            Ok(id) => id,
            Err(error) => {
                return defer_publication_error(
                    state,
                    actor,
                    row,
                    p,
                    PublicationStage::Reconcile,
                    error,
                    None,
                )
                .await;
            }
        }
    } else {
        let snapshot = interface_snapshot(state, p).await?;
        let bytes = publication::package_with_snapshot(
            &body,
            &p.operation_id,
            &p.name,
            &p.version,
            snapshot.as_ref(),
        )?;
        if publication::hash(&bytes) != p.sha256 {
            return Err(conflict());
        }
        set_stage(state, row, p, PublicationStage::FormatValidate.as_str()).await?;
        if let Err(error) = publication::validate(reader, &bytes).await {
            return defer_publication_error(
                state,
                actor,
                row,
                p,
                PublicationStage::FormatValidate,
                error,
                None,
            )
            .await;
        }
        if let Some(base) = &body.base_skill {
            set_stage(state, row, p, PublicationStage::BaseVerify.as_str()).await?;
            if let Err(error) = publication::verify_base(reader, actor, base).await {
                return defer_publication_error(
                    state,
                    actor,
                    row,
                    p,
                    PublicationStage::BaseVerify,
                    error,
                    None,
                )
                .await;
            }
        }
        recheck(state, actor, row, p, &body, PublicationStage::BaseVerify).await?;
        // Durable uncertainty boundary, before the first potentially effective
        // write. Crash/timeout from here on permits reconciliation reads only.
        mark_started(state, row, p).await?;
        match publication::publish_classified(reader, body.base_skill.as_ref(), bytes).await {
            publication::PublishOutcome::Published(id) => id,
            publication::PublishOutcome::Refused { code, status } => {
                defer(
                    state,
                    actor,
                    row,
                    p,
                    DeferredFailure {
                        code,
                        stage: PublicationStage::Publish,
                        status: status.map(i32::from),
                        definitive: true,
                    },
                )
                .await?;
                return Err(AppError::Conflict(code.as_str().into()));
            }
            publication::PublishOutcome::VersionConflict { status } => {
                match publication::reconcile(reader, actor, p, body.base_skill.as_ref()).await {
                    Ok(id) => id,
                    Err(error) => {
                        return defer_publication_error(
                            state,
                            actor,
                            row,
                            p,
                            PublicationStage::Reconcile,
                            error,
                            Some(status),
                        )
                        .await;
                    }
                }
            }
            publication::PublishOutcome::Uncertain { status } => {
                defer(
                    state,
                    actor,
                    row,
                    p,
                    DeferredFailure {
                        code: publication::FailureCode::PublishUncertain,
                        stage: PublicationStage::Publish,
                        status: status.map(i32::from),
                        definitive: false,
                    },
                )
                .await?;
                return Err(publication::ambiguous());
            }
        }
    };
    set_stage(state, row, p, PublicationStage::Verify.as_str()).await?;
    let preview = match publication::verify(reader, actor, p, &id).await {
        Ok(preview) => preview,
        Err(error) => {
            return defer_publication_error(
                state,
                actor,
                row,
                p,
                PublicationStage::Verify,
                error,
                None,
            )
            .await;
        }
    };
    let checkpoint = state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(
        lease_filter(row, p),
        doc! {"$set":{"publication.skill_id":&id,"publication.verified_at":bson::DateTime::now(),"publication.last_stage":"verified"}},
    ).await?;
    if checkpoint.modified_count != 1 {
        return Err(conflict());
    }
    if let Some(target) = target_id(p) {
        state
            .db
            .collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME)
            .update_one(
                doc! {"_id":target,"operation_id":&p.operation_id,"package_sha256":&p.sha256},
                doc! {"$set":{"state":"landed","updated_at":bson::DateTime::now()}},
            )
            .await?;
    }
    set_stage(state, row, p, PublicationStage::Pin.as_str()).await?;
    let (agent, withheld) = recheck(state, actor, row, p, &body, PublicationStage::Pin).await?;
    if let Some(code) = withheld {
        // Observing the landed version is safe; attaching it is not.
        defer(
            state,
            actor,
            row,
            p,
            DeferredFailure {
                code,
                stage: PublicationStage::Pin,
                status: None,
                definitive: false,
            },
        )
        .await?;
        return Err(AppError::Conflict(
            if code == PublicationFailureCode::BaseChanged {
                "The source skill changed; the verified version was not attached"
            } else {
                "Learning evidence or consent is no longer available; the verified version was not attached"
            }
            .into(),
        ));
    }
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
                        doc! {"$set":{"status":"published_unpinned"}},
                    )
                    .session(&mut *session)
                    .await?;
                if marked.matched_count != 1 {
                    return Err(conflict());
                }
                if row_for_pin.source == ProposalSource::Learned {
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

#[path = "assistant_learning_recovery.rs"]
mod recovery;
pub use recovery::{StaleRelease, release_stale, reprepare};

/// First-party human preview of the exact package bound to an authored card.
/// Without a card it keeps the original response; card actions are additive.
pub async fn authored_preview(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    id: &str,
) -> AppResult<Value> {
    let (agent, row) = load(&state.db, actor, agent_id, id).await?;
    Ok(preview_value(state, &agent, &row).await?.0)
}

/// The preview and, when it decoded, the draft it was built from.
async fn preview_value(
    state: &AppState,
    agent: &AssistantAgent,
    row: &AssistantAgentLearningProposal,
) -> AppResult<(Value, Option<GeneratedProposal>)> {
    if row.source != ProposalSource::Authored {
        return Err(not_found());
    }
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    let mut files = Vec::new();
    let mut draft_unavailable = false;
    let mut decoded = None;
    if !matches!(row.status.as_str(), "pinned" | "rejected" | "invalidated") {
        match draft(state, row).await {
            Ok(body) => {
                match interface_snapshot(state, p).await.and_then(|snapshot| {
                    publication::skill_markdown_with_snapshot(
                        &body,
                        &p.operation_id,
                        &p.name,
                        &p.version,
                        snapshot.as_ref(),
                    )
                }) {
                    Ok(markdown) => {
                        files.push(json!({"path":"SKILL.md","content":markdown}));
                        files.extend(
                            body.files
                                .iter()
                                .map(|f| json!({"path":f.path,"content":f.content})),
                        );
                    }
                    Err(_) => draft_unavailable = true,
                }
                decoded = Some(body);
            }
            Err(_) => draft_unavailable = true,
        }
    }
    let failure_code = row.failure_code.as_deref();
    Ok((
        json!({"id":row.id,"agent_id":row.agent_id,"agent_name":agent.display_name.as_deref().unwrap_or(&agent.name),"revision":row.revision,"skills_revision":p.skills_revision,
        "current_skills_revision":agent.skills_revision,"status":shown_status(row),"name":p.name,"version":p.version,"files":files,
        "failure_code":if draft_unavailable { Some("draft_unavailable") } else { failure_code },
        "lease_live":p.lease_expires_at.is_some_and(|t| t > Utc::now()),"base_scripts_not_copied":p.target_kind.as_deref()==Some("update")}),
        decoded,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredAction {
    Publish,
    Deny,
    Retry,
    Check,
    Renew,
    Discard,
    DismissCard,
    Reprepare,
    ShowUpdatedDraft,
}

pub struct AuthoredCardActions {
    pub actions: Vec<AuthoredAction>,
    pub state: &'static str,
    /// Set when another operation's target reservation is the only reason
    /// publish/retry is withheld; shown instead of the stored code.
    pub blocked_by: Option<PublicationFailureCode>,
}

impl AuthoredCardActions {
    pub fn contains(&self, action: AuthoredAction) -> bool {
        self.actions.contains(&action)
    }

    fn settled(state: &'static str) -> Self {
        Self {
            actions: Vec::new(),
            state,
            blocked_by: None,
        }
    }
}

/// Computes the provider-neutral actions currently safe for one acknowledgement
/// card, from the card's own binding and the durable publication state only.
/// Publish/Retry need a live card at the current skills revision, a package
/// that can succeed unchanged, files, a free target and a non-effective
/// operation; Check needs a dispatched operation and a card that approved it
/// (even after expiry) or can still decide; Renew is offered only when no card
/// can act and no attempt holds a lease. A legacy package that never
/// dispatched offers only Reprepare (any card of the draft, live or expired)
/// and Discard/Deny; after a rebuild, older cards offer ShowUpdatedDraft
/// until the new package has its card.
async fn actions_for_card(
    state: &AppState,
    actor: &str,
    agent: &AssistantAgent,
    row: &AssistantAgentLearningProposal,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
    files_present: bool,
    recovery: Recovery,
) -> AppResult<AuthoredCardActions> {
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    match row.status.as_str() {
        "pinned" => return Ok(AuthoredCardActions::settled("pinned")),
        "rejected" => return Ok(AuthoredCardActions::settled("discarded")),
        "invalidated" => return Ok(AuthoredCardActions::settled("invalidated")),
        _ => {}
    }
    if card.status == "denied" {
        return Ok(AuthoredCardActions::settled("dismissed"));
    }
    let Some(reference) = card.authored_skill.as_ref() else {
        return Err(not_found());
    };
    if card.user_id != actor
        || card.tool_name.as_deref() != Some(TOOL)
        || reference.agent_id != row.agent_id
        || reference.proposal_id != row.id
        || reference.revision != row.revision
        || card.arguments_digest.as_deref()
            != Some(
                acks::arguments_digest(&publication::binding(row, p, reference.skills_revision))
                    .as_str(),
            )
    {
        let mut changed = AuthoredCardActions::settled("changed");
        if card.user_id == actor
            && card.tool_name.as_deref() == Some(TOOL)
            && reference.agent_id == row.agent_id
            && reference.proposal_id == row.id
            && updated_draft_unannounced(row)
        {
            changed.actions.push(AuthoredAction::ShowUpdatedDraft);
        }
        return Ok(changed);
    }
    let now = Utc::now();
    let effective = !non_effective(p, now);
    let lease_live = p.lease_expires_at.is_some_and(|expiry| expiry > now);
    let current_revision = reference.skills_revision == agent.skills_revision;
    let card_live = card.expires_at > now && card.status != "expired";
    // A legacy package can never be dispatched; reconciling a dispatched one
    // still uses the ordinary Check.
    let undispatched_legacy = recovery.legacy && !effective;
    let package_publishable = !review_required_failure(row) && !undispatched_legacy;
    let target_available = if let Some(target) = target_id(p) {
        state
            .db
            .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
            .find_one(doc! {"_id":target})
            .await?
            .is_none_or(|holder| {
                holder.operation_id == p.operation_id && holder.package_sha256 == p.sha256
            })
    } else {
        true
    };
    let same_approval = p.approved_by.as_deref() == Some(actor)
        && p.acknowledgement_id.as_deref() == Some(card.id.as_str())
        && p.approval_digest == card.arguments_digest;
    let can_use_decision = card.status == "pending" || card.status == "allowed" || same_approval;
    let mut actions = Vec::new();
    if card.status == "pending" && card_live {
        actions.push(if effective {
            AuthoredAction::DismissCard
        } else {
            AuthoredAction::Deny
        });
    }
    let write = match card.status.as_str() {
        "pending" => Some(AuthoredAction::Publish),
        "allowed" => Some(AuthoredAction::Retry),
        "used" if same_approval => Some(AuthoredAction::Retry),
        _ => None,
    }
    .filter(|_| {
        current_revision && card_live && package_publishable && files_present && !effective
    });
    let mut blocked_by = None;
    match write {
        Some(action) if target_available => actions.push(action),
        Some(_) => blocked_by = Some(PublicationFailureCode::TargetBusy),
        None => {}
    }
    if reprepare_offered(row, recovery, now) {
        actions.push(AuthoredAction::Reprepare);
    }
    if current_revision
        && effective
        && !lease_live
        && !attach_refused(row)
        && (same_approval || (card_live && can_use_decision))
    {
        actions.push(AuthoredAction::Check);
    }
    if !effective && (card.status != "pending" || !card_live) {
        actions.push(AuthoredAction::Discard);
    }
    if package_publishable
        && (!card_live || !current_revision)
        && !lease_live
        && !actions.iter().any(|action| {
            matches!(
                action,
                AuthoredAction::Publish | AuthoredAction::Retry | AuthoredAction::Check
            )
        })
    {
        // Any card at the live binding that can still decide, or the card
        // that approved this operation, makes a renewal redundant.
        let live_digest =
            acks::arguments_digest(&publication::binding(row, p, agent.skills_revision));
        let mut can_act = vec![
            doc! {"status":{"$in":["pending","allowed"]},"expires_at":{"$gt":bson::DateTime::now()}},
        ];
        if let Some(approving) = p.acknowledgement_id.as_deref() {
            can_act.push(doc! {"_id":approving,"status":"used"});
        }
        let another_card = state.db.collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .find_one(doc! {"_id":{"$ne":&card.id},"user_id":actor,"tool_name":TOOL,"authored_skill.proposal_id":&row.id,"arguments_digest":live_digest,"$or":can_act})
            .await?.is_some();
        if !another_card {
            actions.push(AuthoredAction::Renew);
        }
    }
    Ok(AuthoredCardActions {
        actions,
        state: "active",
        blocked_by,
    })
}

/// A rebuilt package whose review card was not raised yet (the second phase
/// of a rebuild did not finish).
fn updated_draft_unannounced(row: &AssistantAgentLearningProposal) -> bool {
    row.publication.as_ref().is_some_and(|p| {
        p.review_card_pending
            && p.package_format >= PACKAGE_FORMAT_SNAPSHOT
            && non_effective(p, Utc::now())
    }) && undispatched_status(row)
}

struct CardView {
    agent: AssistantAgent,
    row: AssistantAgentLearningProposal,
    preview: Value,
    actions: AuthoredCardActions,
    recovery: Recovery,
}

async fn load_for_card(
    state: &AppState,
    actor: &str,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
) -> AppResult<CardView> {
    let reference = card.authored_skill.as_ref().ok_or_else(not_found)?;
    let (agent, row) = load(
        &state.db,
        actor,
        &reference.agent_id,
        &reference.proposal_id,
    )
    .await?;
    let (preview, body) = preview_value(state, &agent, &row).await?;
    let files_present = preview["files"]
        .as_array()
        .is_some_and(|files| !files.is_empty());
    let recovery = recovery(&state.db, &agent, &row, body.as_ref()).await?;
    let actions =
        actions_for_card(state, actor, &agent, &row, card, files_present, recovery).await?;
    Ok(CardView {
        agent,
        row,
        preview,
        actions,
        recovery,
    })
}

pub async fn authored_actions(
    state: &AppState,
    actor: &str,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
) -> AppResult<AuthoredCardActions> {
    Ok(load_for_card(state, actor, card).await?.actions)
}

pub async fn authored_preview_for_card(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    id: &str,
    acknowledgement_id: &str,
) -> AppResult<Value> {
    let card = state.db.collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":acknowledgement_id,"user_id":actor,"tool_name":TOOL,"authored_skill.proposal_id":id,"authored_skill.agent_id":agent_id})
        .await?.ok_or_else(not_found)?;
    let CardView {
        row,
        mut preview,
        actions: computed,
        recovery,
        ..
    } = load_for_card(state, actor, &card).await?;
    if let Some(code) = computed.blocked_by {
        preview["failure_code"] = json!(code.as_str());
    } else if preview["failure_code"] != "draft_unavailable" {
        let shown = shown_failure_code(&row, recovery, Utc::now());
        let expired_but_actionable = shown
            == Some(PublicationFailureCode::ApprovalExpired.as_str())
            && computed.actions.iter().any(|action| {
                matches!(
                    action,
                    AuthoredAction::Publish | AuthoredAction::Retry | AuthoredAction::Check
                )
            });
        preview["failure_code"] = if expired_but_actionable {
            Value::Null
        } else {
            json!(shown)
        };
    }
    preview["actions"] = json!(computed.actions);
    preview["state"] = json!(computed.state);
    Ok(preview)
}

/// Metadata-only audit of a card decision the server withheld.
pub async fn audit_withheld_decision(
    db: &Database,
    actor: &str,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
    computed: &AuthoredCardActions,
    allow: bool,
) {
    let reference = card.authored_skill.as_ref();
    let _ = super::audit_service::log_actor_event(db.clone(), &super::audit_service::AuditActor {user_id:actor.into(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},"assistant_learning_publication_decision_withheld",
        Some(json!({"agent_id":reference.map(|r| &r.agent_id),"proposal_id":reference.map(|r| &r.proposal_id),"revision":reference.map(|r| r.revision),
            "acknowledgement_id":card.id,"card_status":card.status,"decision":if allow {"allow"} else {"deny"},"state":computed.state,"actions":computed.actions,
            "blocked_by":computed.blocked_by.map(PublicationFailureCode::as_str)}))).await;
}

pub async fn record_pre_claim_failure(
    state: &AppState,
    actor: &str,
    reference: &crate::models::assistant_acknowledgement::AuthoredSkillReview,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
    expected_attempt: i64,
    code: PublicationFailureCode,
) -> AppResult<()> {
    if card.user_id != actor
        || card.authored_skill.as_ref().is_none_or(|bound| {
            bound.agent_id != reference.agent_id
                || bound.proposal_id != reference.proposal_id
                || bound.revision != reference.revision
                || bound.skills_revision != reference.skills_revision
        })
    {
        return Err(conflict());
    }
    let row = state.db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&reference.proposal_id,"agent_id":&reference.agent_id,"owner_id":actor,"revision":revision_filter(reference.revision)})
        .await?.ok_or_else(conflict)?;
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    if p.attempt != expected_attempt {
        return Err(conflict());
    }
    let digest = acks::arguments_digest(&publication::binding(&row, p, reference.skills_revision));
    if card.arguments_digest.as_deref() != Some(digest.as_str()) {
        return Err(conflict());
    }
    // A pre-claim refusal never replaces a substantive outcome of an attempt:
    // only an empty, pre-claim or transient code, and never a package refusal.
    let replaceable =
        PublicationFailureCode::codes(PublicationFailureCode::replaceable_before_claim);
    let changed = state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(
        doc! {"_id":&row.id,"revision":revision_filter(row.revision),"publication.operation_id":&p.operation_id,"publication.attempt":attempt_filter(expected_attempt),"publication.lease_expires_at":bson::Bson::Null,"status":{"$in":["pending","publication_failed"]},
            "$and":[{"failure_code":{"$nin":package_refusal_codes()}},{"$or":[{"failure_code":bson::Bson::Null},{"publication.last_stage":"pre_claim"},{"failure_code":{"$in":replaceable}}]}]},
        doc! {"$set":{"status":"publication_failed","failure_code":code.as_str(),"publication.last_stage":"pre_claim","updated_at":bson::DateTime::now()}},
    ).await?;
    if changed.modified_count == 1 {
        audit_deferred(
            &state.db,
            actor,
            &row,
            p,
            DeferredAudit {
                code: code.as_str(),
                stage: "pre_claim",
                status: None,
                clicked: Some(&card.id),
            },
        )
        .await;
    }
    Ok(())
}

pub async fn publication_attempt(
    state: &AppState,
    actor: &str,
    reference: &crate::models::assistant_acknowledgement::AuthoredSkillReview,
) -> AppResult<i64> {
    let row = state.db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&reference.proposal_id,"agent_id":&reference.agent_id,"owner_id":actor,"revision":revision_filter(reference.revision)})
        .await?.ok_or_else(conflict)?;
    Ok(row.publication.as_ref().ok_or_else(conflict)?.attempt)
}

pub async fn target_busy_for(
    state: &AppState,
    actor: &str,
    reference: &crate::models::assistant_acknowledgement::AuthoredSkillReview,
) -> AppResult<bool> {
    let row = state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(
            doc! {"_id":&reference.proposal_id,"agent_id":&reference.agent_id,"owner_id":actor,
            "revision":revision_filter(reference.revision)},
        )
        .await?
        .ok_or_else(conflict)?;
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    let Some(target) = target_id(p) else {
        return Ok(false);
    };
    Ok(state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
        .find_one(doc! {"_id":target})
        .await?
        .is_some_and(|holder| {
            holder.operation_id != p.operation_id || holder.package_sha256 != p.sha256
        }))
}
