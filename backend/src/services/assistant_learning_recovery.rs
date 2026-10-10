//! Recovery of update packages prepared before interface snapshots (#1828).
//! The owner rebuilds a legacy package that never dispatched; an admin may
//! release a stale legacy dispatch on absence-only evidence, after which the
//! owner rebuilds it too. Registry reads go through
//! `assistant_learning_publication`; nothing here speaks Ornn's protocol.
use super::*;
use crate::{
    models::assistant_acknowledgement::{
        AssistantAcknowledgement, COLLECTION_NAME as ACKNOWLEDGEMENTS_COLLECTION_NAME,
    },
    services::audit_service::{self, AuditActor},
};

fn not_rebuildable() -> AppError {
    AppError::Conflict("This draft cannot be rebuilt now; reload it for its current state".into())
}

fn target_unresolved() -> AppError {
    AppError::Conflict("legacy_target_unresolved".into())
}

/// Owner-driven rebuild of a legacy update package. An authored draft is
/// rebuilt from any of its cards (live or expired) and its new card is
/// raised in that card's conversation; a learned draft is rebuilt from the
/// review panel and confirmed there as usual. Calling again after an
/// interrupted rebuild only raises the missing card.
pub async fn reprepare<R: OrnnReader>(
    state: &AppState,
    owner: &AuditActor,
    agent_id: &str,
    proposal_id: &str,
    acknowledgement_id: Option<&str>,
    reader: &R,
) -> AppResult<Value> {
    let actor = owner.user_id.as_str();
    let Some(card_id) = acknowledgement_id else {
        return reprepare_learned(state, owner, agent_id, proposal_id, reader).await;
    };
    let card = state
        .db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .find_one(doc! {"_id":card_id,"user_id":actor,"tool_name":TOOL,
        "authored_skill.proposal_id":proposal_id,"authored_skill.agent_id":agent_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Skill review not found".into()))?;
    let chat =
        crate::services::assistant_skill_authoring::card_authority(&state.db, actor, &card).await?;
    let view = load_for_card(state, actor, &card).await?;
    if view.actions.contains(AuthoredAction::ShowUpdatedDraft) {
        return announce(state, &view.agent, &view.row, &chat).await;
    }
    if !view.actions.contains(AuthoredAction::Reprepare) {
        return Err(not_rebuildable());
    }
    let body = draft(state, &view.row).await?;
    if let Err(error) = rebuild(
        state,
        owner,
        &view.agent,
        &view.row,
        &body,
        Some(&card.id),
        reader,
    )
    .await
    {
        // A concurrent rebuild of the same draft may have won the swap.
        let (agent, row) = load(&state.db, actor, agent_id, proposal_id).await?;
        if matches!(error, AppError::Conflict(_)) && rebuilt(&row) {
            return announce(state, &agent, &row, &chat).await;
        }
        return Err(error);
    }
    let (agent, row) = load(&state.db, actor, agent_id, proposal_id).await?;
    match announce(state, &agent, &row, &chat).await {
        Ok(result) => Ok(result),
        // The rebuild committed; only its card is missing. Older cards offer
        // Show updated draft until it exists.
        Err(error) if updated_draft_unannounced(&row) => {
            tracing::warn!(%error, proposal_id = %row.id, "Rebuilt skill package card was not raised");
            Ok(
                json!({"status":"reprepared","card_pending":true,"proposal_id":row.id,"agent_id":row.agent_id}),
            )
        }
        Err(error) => Err(error),
    }
}

async fn reprepare_learned<R: OrnnReader>(
    state: &AppState,
    owner: &AuditActor,
    agent_id: &str,
    proposal_id: &str,
    reader: &R,
) -> AppResult<Value> {
    let actor = owner.user_id.as_str();
    let (agent, row) = load(&state.db, actor, agent_id, proposal_id).await?;
    if row.source == ProposalSource::Authored {
        return Err(AppError::Conflict(
            "Rebuild this authored skill from its conversation card".into(),
        ));
    }
    if agent.user_id != actor {
        return Err(AppError::Conflict("owner_binding_unavailable".into()));
    }
    let body = draft(state, &row).await?;
    let recovery = recovery(&state.db, &agent, &row, Some(&body)).await?;
    if !reprepare_offered(&row, recovery, Utc::now()) {
        return Err(not_rebuildable());
    }
    rebuild(state, owner, &agent, &row, &body, None, reader).await?;
    Ok(
        json!({"status":"reprepared","proposal_id":row.id,"agent_id":row.agent_id,"revision":row.revision+1}),
    )
}

/// Phase one: replace the legacy operation with a snapshot-aware one built
/// from the same draft and the base read through the owner's identity, in
/// one transaction fenced on the exact undispatched legacy state. The old
/// cards stop matching (new revision and package).
async fn rebuild<R: OrnnReader>(
    state: &AppState,
    owner: &AuditActor,
    agent: &AssistantAgent,
    row: &AssistantAgentLearningProposal,
    body: &GeneratedProposal,
    clicked: Option<&str>,
    reader: &R,
) -> AppResult<()> {
    let actor = owner.user_id.as_str();
    if agent.user_id != actor || row.owner_id != actor {
        return Err(AppError::Conflict("owner_binding_unavailable".into()));
    }
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    // The operation's target and version come from its own encrypted draft
    // and must agree with anything already recorded.
    let base = body.base_skill.as_ref().ok_or_else(target_unresolved)?;
    if p.target_kind
        .as_deref()
        .is_some_and(|kind| kind != "update")
        || p.target_skill_id
            .as_deref()
            .is_some_and(|id| id != base.skill_id)
        || publication::next_version(Some(base)).ok().as_deref() != Some(p.version.as_str())
        || p.name != base.name
    {
        return Err(target_unresolved());
    }
    require_current(&state.db, agent, row).await?;
    if !base_current(&state.db, agent, body, row.source).await? {
        record_rebuild_refusal(
            state,
            actor,
            row,
            PublicationFailureCode::BaseChanged,
            clicked,
        )
        .await?;
        return Err(AppError::Conflict("base_skill_changed".into()));
    }
    let snapshot = match publication::snapshot(reader, actor, base).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let code = error.failure_code(PublicationStage::BaseVerify);
            if NOT_REBUILDABLE.contains(&code) {
                record_rebuild_refusal(state, actor, row, code, clicked).await?;
            }
            return Err(match code {
                PublicationFailureCode::OrnnUnavailable | PublicationFailureCode::NyxidRefused => {
                    AppError::Conflict("source_skill_unavailable: NyxID could not read the source skill from Ornn. Approve any pending Ornn request, then try again.".into())
                }
                _ => error.into_app_error(),
            });
        }
    };
    let operation = Uuid::new_v4().to_string();
    let bytes =
        publication::package_with_snapshot(body, &operation, &p.name, &p.version, Some(&snapshot))?;
    let interface = state
        .encryption_keys
        .encrypt(&serde_json::to_vec(&snapshot).map_err(|_| conflict())?)
        .await?;
    let next = LearningPublication {
        operation_id: operation.clone(),
        name: p.name.clone(),
        version: p.version.clone(),
        sha256: publication::hash(&bytes),
        skills_revision: agent.skills_revision,
        target_kind: Some("update".into()),
        target_skill_id: Some(base.skill_id.clone()),
        interface_encrypted: Some(interface),
        package_format: PACKAGE_FORMAT_SNAPSHOT,
        review_card_pending: row.source == ProposalSource::Authored,
        ..Default::default()
    };
    let next = bson::to_bson(&next).map_err(|_| conflict())?;
    let target = format!("{}:{}", base.skill_id, p.version);
    let event = json!({"agent_id":row.agent_id,"owner_id":row.owner_id,"proposal_id":row.id,
        "revision":row.revision+1,"previous_operation_id":p.operation_id,"operation_id":operation,
        "acknowledgement_id":clicked});
    let mut filter = legacy_package_filter();
    filter.extend(undispatched_status_filter());
    filter.extend(doc! {"_id":&row.id,"owner_id":actor,"revision":revision_filter(row.revision),
        "publication.operation_id":&p.operation_id,"publication.sha256":&p.sha256,
        "publication.attempt":attempt_filter(p.attempt),
        "publication.target_kind":{"$in":[bson::Bson::Null,bson::Bson::String("update".into())]},
        "publication.target_skill_id":{"$in":[bson::Bson::Null,bson::Bson::String(base.skill_id.clone())]},
        "publication.started":{"$ne":true},"publication.uncertain_dispatch":{"$ne":true},
        "publication.verified_at":bson::Bson::Null,
        "$or":[{"publication.lease_expires_at":bson::Bson::Null},{"publication.lease_expires_at":{"$lte":bson::DateTime::now()}}]});
    let db = state.db.clone();
    let audit_key = state.audit_chain_hmac_key.clone();
    let owner = owner.clone();
    let (old_operation, old_sha, new_revision) =
        (p.operation_id.clone(), p.sha256.clone(), row.revision + 1);
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                // Only this operation's own unused reservation; an uncertain
                // or landed target means it dispatched and the swap fails.
                db.collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME)
                    .delete_one(doc! {"_id":&target,"operation_id":&old_operation,
                    "package_sha256":&old_sha,"state":"reserved"})
                    .session(&mut *session)
                    .await?;
                let changed = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(
                        filter.clone(),
                        doc! {"$set":{"publication":next.clone(),"status":"pending",
                        "failure_code":bson::Bson::Null,"revision":new_revision,
                        "updated_at":bson::DateTime::now()}},
                    )
                    .session(&mut *session)
                    .await?;
                if changed.modified_count != 1 {
                    return Err(conflict());
                }
                audit_service::log_actor_event_in_session(
                    &db,
                    session,
                    audit_key.as_slice(),
                    &owner,
                    "assistant_learning_proposal_reprepared",
                    event.clone(),
                )
                .await
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// A base the rebuild cannot reproduce is definitive for an undispatched
/// legacy package; record why, so its cards stop offering the rebuild.
async fn record_rebuild_refusal(
    state: &AppState,
    actor: &str,
    row: &AssistantAgentLearningProposal,
    code: PublicationFailureCode,
    clicked: Option<&str>,
) -> AppResult<()> {
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    let mut filter = legacy_package_filter();
    filter.extend(undispatched_status_filter());
    filter.extend(doc! {"_id":&row.id,"revision":revision_filter(row.revision),
        "publication.operation_id":&p.operation_id,"publication.started":{"$ne":true},
        "publication.uncertain_dispatch":{"$ne":true},"publication.verified_at":bson::Bson::Null,
        "$or":[{"publication.lease_expires_at":bson::Bson::Null},{"publication.lease_expires_at":{"$lte":bson::DateTime::now()}}]});
    let changed = state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            filter,
            doc! {"$set":{"status":"publication_failed","failure_code":code.as_str(),
            "publication.last_stage":"reprepare","updated_at":bson::DateTime::now()}},
        )
        .await?;
    if changed.modified_count == 1 {
        audit_deferred(
            &state.db,
            actor,
            row,
            p,
            DeferredAudit {
                code: code.as_str(),
                stage: "reprepare",
                status: None,
                clicked,
            },
        )
        .await;
    }
    Ok(())
}

fn rebuilt(row: &AssistantAgentLearningProposal) -> bool {
    row.publication
        .as_ref()
        .is_some_and(|p| p.package_format >= PACKAGE_FORMAT_SNAPSHOT)
}

/// Phase two: raise the review card for the rebuilt package in the clicked
/// card's conversation, once; a live card at that binding (for example one a
/// concurrent click raised) is returned instead. It authorizes nothing; that
/// card decides publication.
async fn announce(
    state: &AppState,
    agent: &AssistantAgent,
    row: &AssistantAgentLearningProposal,
    chat: &ChatAuthority,
) -> AppResult<Value> {
    if !rebuilt(row) || row.owner_id != chat.user_id || agent.user_id != chat.user_id {
        return Err(not_rebuildable());
    }
    let p = row.publication.as_ref().ok_or_else(conflict)?;
    let binding = publication::binding(row, p, agent.skills_revision);
    let existing = state
        .db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .find_one(doc! {"user_id":&chat.user_id,"tool_name":TOOL,"authored_skill.proposal_id":&row.id,
            "arguments_digest":acks::arguments_digest(&binding),"status":{"$in":["pending","allowed"]},
            "expires_at":{"$gt":bson::DateTime::now()}})
        .await?;
    let card = match existing {
        Some(card) => card,
        None if updated_draft_unannounced(row) => {
            acks::request(&state.db, chat, acks::Request {
                kind: "action", service: None, tool: Some(TOOL), arguments: Some(&binding),
                summary: "Review the rebuilt skill package, publish it privately to Ornn and attach its exact version to this agent.",
                platform: false,
            })
            .await?
        }
        None => return Err(not_rebuildable()),
    };
    state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id,"revision":revision_filter(row.revision),
            "publication.operation_id":&p.operation_id,"publication.review_card_pending":true},
            doc! {"$set":{"publication.review_card_pending":false}},
        )
        .await?;
    let mut result = acks::refusal(&card);
    result["status"] = "reprepared".into();
    result["proposal_id"] = row.id.clone().into();
    result["agent_id"] = row.agent_id.clone().into();
    Ok(result)
}

pub struct StaleRelease {
    pub dry_run: bool,
    pub min_age_hours: i64,
    pub limit: usize,
}

const STALE_RELEASE_MIN_AGE_HOURS: i64 = 24;
const STALE_RELEASE_MAX_AGE_HOURS: i64 = 24 * 365;
const STALE_RELEASE_MAX_ROWS: usize = 200;
const STALE_RELEASE_CONCURRENCY: usize = 4;
const STALE_RELEASE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
const ABSENCE_ONLY: &str = "absence_only";
const STALE_RELEASE_RESIDUAL: &str = "Absence-only evidence: a request stalled longer than min_age_hours can still land after release. A rebuilt package pins only after exact-version and ZIP-hash verification; a late landing makes it version_conflict.";
/// Drafts discarded before the migration whose dispatched operation still
/// holds its target. Only that reservation is released; they stay discarded.
const DISCARDED_STATUSES: [&str; 2] = ["rejected", "invalidated"];

#[derive(Serialize)]
pub struct StaleReleaseRow {
    pub proposal_id: String,
    pub operation_id: String,
    pub target: Option<String>,
    pub decision: String,
    pub evidence: &'static str,
}

/// Admin release of stale legacy dispatches. A candidate is a dispatched,
/// unverified, unleased legacy update older than `min_age_hours` (measured
/// from the latest of first dispatch, startup classification and lease
/// expiry) whose target it still holds. Its exact version must be a
/// definitive 404 and, read with the proposal owner's identity now, the
/// latest version must be its exact base (a discarded draft has no draft
/// left, so its version must instead be missing from the version list). A
/// dry run changes nothing. Each release commits with its audit event.
pub async fn release_stale<R, F>(
    state: &AppState,
    operator: &AuditActor,
    request: StaleRelease,
    reader_for: F,
) -> AppResult<Value>
where
    R: OrnnReader + Send,
    F: Fn(&str) -> R + Sync,
{
    let StaleRelease {
        dry_run,
        min_age_hours,
        limit,
    } = request;
    if !(STALE_RELEASE_MIN_AGE_HOURS..=STALE_RELEASE_MAX_AGE_HOURS).contains(&min_age_hours)
        || !(1..=STALE_RELEASE_MAX_ROWS).contains(&limit)
    {
        return Err(AppError::ValidationError(
            "min_age_hours must be 24..=8760 and limit 1..=200".into(),
        ));
    }
    let deadline = tokio::time::Instant::now() + STALE_RELEASE_DEADLINE;
    let cutoff = bson::DateTime::from_chrono(Utc::now() - Duration::hours(min_age_hours));
    let (aged, unstamped) = stale_candidate_filters(cutoff);
    let proposals = state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
    let mut rows: Vec<AssistantAgentLearningProposal> = proposals
        .find(aged)
        .sort(doc! {"publication.legacy_classified_at":1,"_id":1})
        .limit(limit as i64)
        .await?
        .try_collect()
        .await?;
    // Rows the startup stamp missed are reported after every releasable
    // row, never released, and never displace one.
    if rows.len() < limit {
        let mut more: Vec<AssistantAgentLearningProposal> = proposals
            .find(unstamped)
            .sort(doc! {"_id":1})
            .limit((limit - rows.len()) as i64)
            .await?
            .try_collect()
            .await?;
        rows.append(&mut more);
    }
    let batch_id = Uuid::new_v4().to_string();
    let reader_for = &reader_for;
    let mut results: Vec<(usize, StaleReleaseRow)> =
        futures::stream::iter(rows.into_iter().enumerate())
            .map(|(index, row)| {
                let batch_id = batch_id.as_str();
                async move {
                    let decision = stale_decision(
                        state,
                        operator,
                        &row,
                        reader_for,
                        dry_run,
                        min_age_hours,
                        batch_id,
                        deadline,
                    )
                    .await;
                    let p = row.publication.as_ref();
                    (
                        index,
                        StaleReleaseRow {
                            proposal_id: row.id.clone(),
                            operation_id: p.map(|p| p.operation_id.clone()).unwrap_or_default(),
                            target: p.and_then(target_id),
                            decision,
                            evidence: ABSENCE_ONLY,
                        },
                    )
                }
            })
            .buffer_unordered(STALE_RELEASE_CONCURRENCY)
            .collect()
            .await;
    results.sort_by_key(|(index, _)| *index);
    let rows: Vec<StaleReleaseRow> = results.into_iter().map(|(_, row)| row).collect();
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for row in &rows {
        *counts.entry(row.decision.as_str()).or_default() += 1;
    }
    if let Err(error) = audit_service::log_actor_event(
        state.db.clone(),
        operator,
        "assistant_learning_publication_stale_release_summary",
        Some(
            json!({"batch_id":batch_id,"dry_run":dry_run,"min_age_hours":min_age_hours,
            "limit":limit,"decisions":counts}),
        ),
    )
    .await
    {
        tracing::error!(%error, batch_id, "Stale skill publication release summary audit failed");
    }
    Ok(
        json!({"batch_id":batch_id,"dry_run":dry_run,"min_age_hours":min_age_hours,"limit":limit,
        "evidence":ABSENCE_ONLY,"residual":STALE_RELEASE_RESIDUAL,"rows":rows}),
    )
}

/// Stale release candidates: stamped rows older than `cutoff`, and rows the
/// startup stamp missed. Each `$or` branch has an index:
/// `{publication.started, status}` and the partial
/// `{publication.uncertain_dispatch, status}`.
pub(super) fn stale_candidate_filters(cutoff: bson::DateTime) -> (bson::Document, bson::Document) {
    let mut statuses = vec!["pending", "publishing", "publication_failed"];
    statuses.extend(DISCARDED_STATUSES);
    let mut candidate = legacy_package_filter();
    candidate.extend(doc! {
        "status":{"$in":statuses},
        "publication.target_kind":"update",
        "publication.verified_at":bson::Bson::Null,
        "$or":[{"publication.started":true},{"publication.uncertain_dispatch":true}],
    });
    let mut aged = candidate.clone();
    aged.extend(doc! {
        "publication.legacy_classified_at":{"$lte":cutoff},
        "$and":[{"$or":[{"publication.dispatched_at":bson::Bson::Null},{"publication.dispatched_at":{"$lte":cutoff}}]},
            {"$or":[{"publication.lease_expires_at":bson::Bson::Null},{"publication.lease_expires_at":{"$lte":cutoff}}]}],
    });
    candidate.insert("publication.legacy_classified_at", bson::Bson::Null);
    (aged, candidate)
}

#[allow(clippy::too_many_arguments)]
async fn stale_decision<R, F>(
    state: &AppState,
    operator: &AuditActor,
    row: &AssistantAgentLearningProposal,
    reader_for: &F,
    dry_run: bool,
    min_age_hours: i64,
    batch_id: &str,
    deadline: tokio::time::Instant,
) -> String
where
    R: OrnnReader + Send,
    F: Fn(&str) -> R + Sync,
{
    let Some(p) = row.publication.as_ref() else {
        return "skip:cas_changed".into();
    };
    if p.legacy_classified_at.is_none() {
        return "skip:age_unknown".into();
    }
    let discarded = DISCARDED_STATUSES.contains(&row.status.as_str());
    let (Some(target), Some(skill_id)) = (target_id(p), p.target_skill_id.as_deref()) else {
        return "skip:legacy_target_unresolved".into();
    };
    // The base comes from the operation's own draft and must agree with the
    // recorded target. A discarded draft no longer has one.
    let base_version = match draft(state, row).await {
        Ok(body) => match body.base_skill.as_ref().filter(|base| {
            base.skill_id == skill_id
                && publication::next_version(Some(base)).ok().as_deref() == Some(p.version.as_str())
        }) {
            Some(base) => Some(base.version.clone()),
            None => return "skip:legacy_target_unresolved".into(),
        },
        Err(_) if discarded => None,
        Err(_) => return "skip:legacy_target_unresolved".into(),
    };
    match state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
        .find_one(doc! {"_id":&target})
        .await
    {
        Ok(Some(holder))
            if holder.operation_id == p.operation_id
                && holder.package_sha256 == p.sha256
                && matches!(holder.state.as_str(), "reserved" | "uncertain") => {}
        Ok(_) => return "skip:barrier_changed".into(),
        Err(error) => {
            tracing::warn!(%error, proposal_id = %row.id, "Stale release barrier read failed");
            return "skip:uncertain".into();
        }
    }
    let reader = reader_for(&row.owner_id);
    let Ok(evidence) = tokio::time::timeout_at(
        deadline,
        publication::legacy_release_evidence(
            &reader,
            skill_id,
            &p.version,
            base_version.as_deref(),
        ),
    )
    .await
    else {
        return "skip:deadline".into();
    };
    let latest_version = match evidence {
        publication::LegacyReleaseEvidence::Absent { latest } => latest,
        // Nothing of a discarded draft is left to check.
        publication::LegacyReleaseEvidence::Landed if discarded => return "skip:landed".into(),
        publication::LegacyReleaseEvidence::Landed => return "landed_check_again".into(),
        publication::LegacyReleaseEvidence::LatestMismatch => {
            return "skip:latest_mismatch".into();
        }
        publication::LegacyReleaseEvidence::Uncertain => return "skip:uncertain".into(),
    };
    if dry_run {
        return if discarded {
            "release_reservation"
        } else {
            "release"
        }
        .into();
    }
    if tokio::time::Instant::now() >= deadline {
        return "skip:deadline".into();
    }
    let evidence = ReleaseEvidence {
        checked_at: Utc::now(),
        exact_version_status: 404,
        latest_version,
        package_sha256: p.sha256.clone(),
        min_age_hours,
        batch_id: batch_id.into(),
    };
    match release_one(state, operator, row, p, &target, skill_id, evidence).await {
        Ok(()) if discarded => "released_reservation_audited".into(),
        Ok(()) => "released_audited".into(),
        Err(AppError::Conflict(_)) => "skip:cas_changed".into(),
        Err(error) => {
            tracing::error!(%error, proposal_id = %row.id, "Stale skill publication release failed");
            "skip:error".into()
        }
    }
}

/// One stale release: compare-and-swap on the full legacy dispatch state
/// observed for this apply, the target barrier and the audit event commit
/// together. A discarded draft keeps its status; only its reservation goes.
async fn release_one(
    state: &AppState,
    operator: &AuditActor,
    row: &AssistantAgentLearningProposal,
    p: &LearningPublication,
    target: &str,
    target_skill_id: &str,
    evidence: ReleaseEvidence,
) -> AppResult<()> {
    let mut filter = legacy_package_filter();
    filter.extend(doc! {"_id":&row.id,"revision":revision_filter(row.revision),"status":&row.status,
        "publication.operation_id":&p.operation_id,"publication.attempt":attempt_filter(p.attempt),
        "publication.sha256":&p.sha256,"publication.target_kind":"update",
        "publication.target_skill_id":target_skill_id,"publication.verified_at":bson::Bson::Null,
        "$and":[{"$or":[{"publication.started":true},{"publication.uncertain_dispatch":true}]},
            {"$or":[{"publication.lease_expires_at":bson::Bson::Null},{"publication.lease_expires_at":{"$lte":bson::DateTime::now()}}]}]});
    let mut fields = released_fields(&row.status);
    fields.insert(
        "publication.release_evidence",
        bson::to_bson(&evidence).map_err(|_| conflict())?,
    );
    let event = json!({"agent_id":row.agent_id,"owner_id":row.owner_id,"proposal_id":row.id,
        "operation_id":p.operation_id,"target":target,"mode":"stale_bulk","evidence":ABSENCE_ONLY,
        "reservation_only":DISCARDED_STATUSES.contains(&row.status.as_str()),"proposal_status":row.status,
        "evidence_ref":format!("bulk-stale:{}h:{}",evidence.min_age_hours,evidence.checked_at.format("%Y-%m-%d")),
        "batch_id":evidence.batch_id,"checked_at":evidence.checked_at,
        "exact_version_status":evidence.exact_version_status,"latest_version":evidence.latest_version,
        "package_sha256":evidence.package_sha256,"min_age_hours":evidence.min_age_hours});
    let db = state.db.clone();
    let audit_key = state.audit_chain_hmac_key.clone();
    let operator = operator.clone();
    let (target, operation, sha) = (target.to_owned(), p.operation_id.clone(), p.sha256.clone());
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let barrier = db
                    .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
                    .find_one(
                        doc! {"_id":&target,"operation_id":&operation,"package_sha256":&sha,
                        "state":{"$in":["reserved","uncertain"]}},
                    )
                    .session(&mut *session)
                    .await?
                    .ok_or_else(conflict)?;
                let changed = db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(filter.clone(), doc! {"$set":fields.clone()})
                    .session(&mut *session)
                    .await?;
                if changed.modified_count != 1 {
                    return Err(conflict());
                }
                let released = db
                    .collection::<bson::Document>(PUBLICATION_TARGETS_COLLECTION_NAME)
                    .delete_one(doc! {"_id":&target,"operation_id":&operation,
                    "package_sha256":&sha,"state":&barrier.state})
                    .session(&mut *session)
                    .await?;
                if released.deleted_count != 1 {
                    return Err(conflict());
                }
                let mut event = event.clone();
                event["previous_state"] = barrier.state.into();
                audit_service::log_actor_event_in_session(
                    &db,
                    session,
                    audit_key.as_slice(),
                    &operator,
                    "assistant_learning_publication_target_released",
                    event,
                )
                .await
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}
