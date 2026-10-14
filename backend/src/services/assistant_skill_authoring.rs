//! Owner-requested drafts, sharing L1 review, encryption and publication.
use super::{
    assistant_acknowledgement_service::{self as acks, ChatAuthority},
    assistant_agent_learning::{self as learning, GeneratedFile, GeneratedProposal},
    assistant_agent_learning_review as review, assistant_team_service as team,
    feature_flag_service,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_acknowledgement::{
            AssistantAcknowledgement, COLLECTION_NAME as ACKNOWLEDGEMENTS_COLLECTION_NAME,
        },
        assistant_agent_learning::{
            AssistantAgentLearningProposal, PROPOSALS_COLLECTION_NAME, ProposalSource,
        },
    },
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;
use uuid::Uuid;

pub const TOOL_NAME: &str = "draft_agent_skill";
pub const SPECIALIST_GUIDANCE: &str = "\nYour skills: Ornn skills hold your repeatable procedures, checklists, references and templates. To create or improve one, call nyxid__draft_agent_skill with your own agent ID (from nyxid__get_agent_skills); to improve, pass the attached skill as base_skill. Full access publishes immediately; otherwise the owner reviews one card; NyxID packages, validates, publishes privately to Ornn and pins it. Never package or publish skills through raw Ornn upload APIs, sandboxes or machines, and never ask anyone for a ZIP. Never include secrets. End your turn after drafting.";
pub const GUIDANCE: &str = "\nAgents: description defines role/scope; persona defines tone/style; Ornn skills hold repeatable procedures, checklists, domain references, output templates and multi-step workflows. For uncovered procedures, use nyxid__draft_agent_skill only after search_agent_skills, preview_agent_skill and proposing suitable existing skills with set_agent_skills. Full access publishes immediately; other modes need one owner review card; NyxID packages, publishes and pins. Never package or publish skills through Ornn Playground, sandboxes, machines or raw Ornn upload APIs, and never ask anyone for a ZIP. Never include secrets. End your turn after drafting; the owner card publishes without a tool retry.";

/// No membership reads on the default-off or person-only rollout path.
pub async fn enabled(db: &Database, actor: &str) -> AppResult<bool> {
    match feature_flag_service::flag_enabled_people(db, learning::FLAG_KEY).await? {
        Some(people) => Ok(people.iter().any(|person| person == actor)),
        None => feature_flag_service::personal_flag_enabled(db, actor, learning::FLAG_KEY).await,
    }
}

/// NyxBot drafts for any agent it maintains; a specialist only for itself.
/// Guests never draft. Full access publishes directly; other modes need the owner's review card.
pub async fn require_author(db: &Database, chat: &ChatAuthority) -> AppResult<()> {
    if chat.guest {
        return Err(AppError::Forbidden(
            "Guests cannot draft agent skills".into(),
        ));
    }
    if !enabled(db, &chat.user_id).await? {
        return Err(AppError::Forbidden(
            "Agent skill authoring is not enabled".into(),
        ));
    }
    Ok(())
}

/// The live chat that raised an authored card: its key is active, still bound
/// to the card's conversation, and that chat may author skills. Shared by the
/// card decision and its renewal.
pub async fn card_authority(
    db: &Database,
    actor: &str,
    card: &AssistantAcknowledgement,
) -> AppResult<ChatAuthority> {
    source_authority(db, actor, &card.api_key_id, &card.conversation_id).await
}

async fn source_authority(
    db: &Database,
    actor: &str,
    key: &str,
    conversation: &str,
) -> AppResult<ChatAuthority> {
    let live = db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":key,"user_id":actor,"is_active":true,
        "$or":[{"expires_at":bson::Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]})
        .await?
        .is_some();
    let chat = if live {
        acks::for_key(db, actor, Some(key))
            .await?
            .filter(|chat| chat.conversation_id == conversation)
    } else {
        None
    }
    .ok_or_else(|| AppError::Forbidden("Skill review key is no longer current".into()))?;
    require_author(db, &chat).await?;
    Ok(chat)
}

/// The authored card being renewed and its live chat, so the new card lands
/// in the same conversation.
pub async fn renewal_source(
    db: &Database,
    actor: &str,
    agent_id: &str,
    proposal_id: &str,
    original_id: &str,
    revision: Option<i64>,
) -> AppResult<(AssistantAcknowledgement, ChatAuthority)> {
    let original = db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .find_one(doc! {"_id":original_id,"user_id":actor,"tool_name":review::TOOL})
        .await?
        .ok_or_else(|| AppError::NotFound("Skill review not found".into()))?;
    let reference = original
        .authored_skill
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Skill review not found".into()))?;
    if reference.agent_id != agent_id
        || reference.proposal_id != proposal_id
        || revision.is_some_and(|v| v != reference.revision)
    {
        return Err(AppError::Conflict("Skill review changed".into()));
    }
    let chat = card_authority(db, actor, &original).await?;
    Ok((original, chat))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftInput {
    pub agent: String,
    name: String,
    description: String,
    skill_md: String,
    #[serde(default)]
    files: Vec<GeneratedFile>,
    #[serde(default)]
    base_skill: Option<BaseInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BaseInput {
    skill_id: String,
    version: String,
}

#[path = "assistant_skill_authoring_validation.rs"]
pub(crate) mod validation;
use validation::invalid;
pub(crate) use validation::{decode as decode_body, validate as validate_body};

pub(crate) fn parse_input(value: &Value) -> AppResult<DraftInput> {
    // Diagnose only known schema locations. Unknown JSON keys can themselves
    // contain private material, so report their containing object instead.
    validation::object(
        value,
        "draft",
        &[
            "agent",
            "name",
            "description",
            "skill_md",
            "files",
            "base_skill",
        ],
    )?;
    for field in ["agent", "name", "description", "skill_md"] {
        validation::string(&value[field], field)?;
    }
    validation::files_shape(value.get("files"))?;
    if let Some(base) = value.get("base_skill").filter(|v| !v.is_null()) {
        validation::object(base, "base_skill", &["skill_id", "version"])?;
        for field in ["skill_id", "version"] {
            validation::string(&base[field], &format!("base_skill.{field}"))?;
        }
    }
    serde_json::from_value(value.clone()).map_err(|_| invalid("invalid_shape", "draft"))
}

/// `reader` reads as the drafting chat; `validator` sends the pre-review
/// format check (the drafting chat for agents, the person for owner drafts).
pub async fn create(
    state: &AppState,
    chat: &ChatAuthority,
    mut input: DraftInput,
    reader: &impl super::agent_skill_service::OrnnReader,
    validator: &impl super::agent_skill_service::OrnnReader,
) -> AppResult<Value> {
    require_author(&state.db, chat).await?;
    let agent = team::maintained_agent(&state.db, &chat.user_id, &input.agent).await?;
    if agent.destroyed_at.is_some() {
        return Err(AppError::NotFound("Agent not found".into()));
    }
    if !chat.is_orchestrator() && agent.id != chat.agent_id {
        return Err(AppError::Forbidden(
            "Specialists may draft skills only for their own agent; ask NyxBot for others".into(),
        ));
    }
    // The owner reviews the paths Ornn will receive.
    for file in &mut input.files {
        file.path = super::assistant_learning_publication::package_path(&file.path).into_owned();
    }
    if agent.user_id != chat.user_id {
        return Err(AppError::Conflict("owner_binding_unavailable: Ornn currently publishes person-owned skills only. Ask an organization maintainer to attach an existing approved skill; no personal publication fallback is allowed".into()));
    }
    let base_skill = input
        .base_skill
        .map(|base| {
            let pin = agent
                .skills
                .iter()
                .find(|pin| {
                    pin.skill_id == base.skill_id
                        && pin.version == base.version
                        && pin.source == "ornn"
                        && pin.dependencies.is_empty()
                })
                .ok_or_else(|| invalid("base_skill_not_attached", "base_skill"))?;
            if input.name != pin.name {
                return Err(invalid("base_skill_name_mismatch", "name"));
            }
            Ok(crate::models::catalog_skill_revision::SkillPin {
                source: pin.source.clone(),
                skill_id: pin.skill_id.clone(),
                name: pin.name.clone(),
                version: pin.version.clone(),
                sha256: pin.sha256.clone(),
            })
        })
        .transpose()?;
    let body = GeneratedProposal {
        schema_version: 1,
        kind: if base_skill.is_some() {
            "improve"
        } else {
            "new"
        }
        .into(),
        name: input.name,
        description: input.description,
        skill_md: input.skill_md,
        files: input.files,
        base_skill,
        rationale: String::new(),
        safety_notes: String::new(),
    };
    let encoded = validate_body(&body)?;
    validation::ornn_rules(&body)?;
    review::validate_proposal_base(&state.db, &agent, &body, ProposalSource::Authored).await?;
    ornn_preflight(validator, &body).await?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(state.audit_chain_hmac_key.as_slice())
        .expect("HMAC key");
    mac.update(b"nyxid-authored-skill-v1\0");
    mac.update(agent.id.as_bytes());
    mac.update(&encoded);
    let fingerprint = hex::encode(mac.finalize().into_bytes());
    let now = Utc::now();
    let candidate = AssistantAgentLearningProposal {
        source: ProposalSource::Authored,
        id: Uuid::new_v4().to_string(),
        agent_id: agent.id.clone(),
        owner_id: agent.user_id.clone(),
        run_id: String::new(),
        status: "pending".into(),
        revision: 0,
        config_revision: 0,
        agent_skills_revision: agent.skills_revision,
        fingerprint,
        input_digest: String::new(),
        model_contract: "authored-skill-v1".into(),
        publication: None,
        failure_code: None,
        evidence: Vec::new(),
        body_encrypted: state.encryption_keys.encrypt(&encoded).await?,
        body_bytes: encoded.len() as i64,
        created_at: now,
        updated_at: now,
    };
    let reservation = claims::reserve(state, chat, &candidate).await?;
    let automatic = !chat.guest
        && acks::effective_access_mode(&state.db, chat).await?
            == crate::models::assistant_settings::AccessMode::Autopilot;
    if let Some(card) = &reservation.card
        && !automatic
    {
        return Ok(reused_result(
            state,
            card,
            &reservation.proposal.id,
            &agent.id,
        ));
    }
    let id = &reservation.proposal.id;
    let binding = review::approval_binding_typed(
        state,
        &chat.user_id,
        &agent.id,
        id,
        reservation.proposal.revision,
        agent.skills_revision,
        reader,
    )
    .await;
    let binding = match binding {
        Ok(binding) => binding,
        Err(failure) => {
            // Keep the reserved proposal retryable: another caller may be
            // preparing the same generation, or this worker may be retried.
            if body.base_skill.is_some()
                && matches!(
                    failure.code,
                    Some(
                        review::PublicationFailureCode::OrnnUnavailable
                            | review::PublicationFailureCode::NyxidRefused
                    )
                )
            {
                return Err(AppError::Conflict(if automatic {
                    "source_skill_unavailable: NyxID could not read the source skill from Ornn under the current grants and policy."
                } else {
                    "source_skill_unavailable: NyxID could not read the source skill from Ornn. Approve any pending Ornn request, then draft again."
                }.into()));
            }
            return Err(failure.error);
        }
    };
    if automatic {
        return publish_automatically(state, chat, &agent, id, &binding, reader).await;
    }
    let card = claims::finalize(state, &reservation, &binding).await?;
    Ok(reused_result(state, &card, id, &agent.id))
}

#[path = "assistant_skill_authoring_claims.rs"]
mod claims;

/// Ornn checks the exact package before the owner is asked to review it, so
/// a refused format returns to the author as fixable rule identifiers. Only
/// an explicit verdict refuses: an unavailable Ornn or an unreadable base
/// leaves the existing retryable draft flow unchanged, and publication
/// validates again after approval.
async fn ornn_preflight(
    validator: &impl super::agent_skill_service::OrnnReader,
    body: &GeneratedProposal,
) -> AppResult<()> {
    match super::assistant_learning_publication::preflight(validator, body).await {
        Ok(Some(rules)) => Err(
            crate::errors::skill_draft::SkillDraftValidation::ornn_rejected(
                rules.iter().map(String::as_str),
            )
            .into(),
        ),
        Ok(None) | Err(_) => Ok(()),
    }
}

fn reused_result(
    state: &AppState,
    card: &AssistantAcknowledgement,
    proposal: &str,
    agent: &str,
) -> Value {
    let mut result = draft_result(state, card, proposal, agent);
    if card.status == "expired" || card.expires_at <= Utc::now() {
        result["status"] = "expired".into();
        result["instructions"] = "This draft already has an expired review card in the NyxID app. Send review_url as a full URL; the owner can press Request a new confirmation there and review all files. A chat reply never approves publication.".into();
    } else if card.status == "allowed" {
        result["status"] = "waiting_publication".into();
        result["instructions"] = "The owner already allowed this exact draft. Publication is pending on the existing review card; send review_url as a full URL to inspect its status. Do not create another draft or confirmation, renew it, or dispatch publication again.".into();
    }
    result
}

fn draft_result(
    state: &AppState,
    card: &AssistantAcknowledgement,
    proposal: &str,
    agent: &str,
) -> Value {
    let mut result = acks::refusal_with_frontend(card, &state.config.frontend_url);
    result["proposal_id"] = proposal.into();
    result["agent_id"] = agent.into();
    result["source"] = "authored".into();
    result
}

/// The owner's "Edit" of an attached skill in the app: an authored improve
/// draft prefilled from the exact pinned package, reviewed on an owner card in
/// `chat` (NyxBot's home thread) like any authored draft. Repeating the same
/// edit reuses the pending draft.
pub async fn create_improve_draft(
    state: &AppState,
    chat: &ChatAuthority,
    agent_id: &str,
    skill_id: &str,
    reader: &impl super::agent_skill_service::OrnnReader,
    validator: &impl super::agent_skill_service::OrnnReader,
) -> AppResult<Value> {
    require_author(&state.db, chat).await?;
    let agent = team::maintained_agent(&state.db, &chat.user_id, agent_id).await?;
    if agent.destroyed_at.is_some() {
        return Err(AppError::NotFound("Agent not found".into()));
    }
    if agent.user_id != chat.user_id {
        return Err(AppError::Conflict("owner_binding_unavailable: Ornn currently publishes person-owned skills only. Ask an organization maintainer to attach an existing approved skill; no personal publication fallback is allowed".into()));
    }
    let pin = agent
        .skills
        .iter()
        .find(|pin| pin.skill_id == skill_id && pin.source == "ornn")
        .cloned()
        .ok_or_else(|| AppError::NotFound("Attached skill not found".into()))?;
    if !pin.dependencies.is_empty() {
        return Err(AppError::Conflict(
            "skill_dependencies_unsupported: skills with pinned dependencies are improved through NyxBot".into(),
        ));
    }
    let mut files = super::agent_skill_service::pinned_files(reader, &pin).await?;
    let skill_md = files
        .remove("SKILL.md")
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| {
            AppError::Conflict("source_skill_unavailable: SKILL.md is not text".into())
        })?;
    let mut extra = Vec::new();
    for (path, bytes) in files {
        let content = String::from_utf8(bytes).ok().filter(|_| {
            (path.ends_with(".md") || path.ends_with(".txt")) && !path.contains(['\\', ':'])
        });
        let Some(content) = content else {
            return Err(AppError::Conflict(
                "skill_files_unsupported: only .md and .txt text files can be edited here; improve this skill through NyxBot".into(),
            ));
        };
        // The owner reviews the paths Ornn will receive.
        let path = super::assistant_learning_publication::package_path(&path).into_owned();
        extra.push(GeneratedFile { path, content });
    }
    let description = agent
        .skill_metadata
        .get(&pin.skill_id)
        .map(|meta| meta.description.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| format!("Improved version of {}", pin.name));
    let body = GeneratedProposal {
        schema_version: 1,
        kind: "improve".into(),
        name: pin.name.clone(),
        description,
        skill_md: without_frontmatter(&skill_md).to_owned(),
        files: extra,
        base_skill: Some(crate::models::catalog_skill_revision::SkillPin {
            source: pin.source.clone(),
            skill_id: pin.skill_id.clone(),
            name: pin.name.clone(),
            version: pin.version.clone(),
            sha256: pin.sha256.clone(),
        }),
        rationale: String::new(),
        safety_notes: String::new(),
    };
    let seed = format!(
        "skill-improve:{}:{}:{}",
        agent.id, pin.skill_id, pin.version
    );
    submit_owner_draft(
        state,
        chat,
        &agent,
        body,
        reader,
        validator,
        |fingerprint| Ok(format!("{seed}:{fingerprint}")),
    )
    .await
}

/// Published SKILL.md starts with NyxID's generated YAML frontmatter; the
/// editable guidance is what follows it.
fn without_frontmatter(text: &str) -> &str {
    text.strip_prefix("---\n")
        .and_then(|rest| rest.find("\n---\n").map(|end| &rest[end + 5..]))
        .map_or(text, |body| body.trim_start_matches('\n'))
}

/// Validate, store (idempotently, keyed by `seed(fingerprint)`) and raise the
/// owner review card for a draft the owner starts in the app. Unlike NyxBot
/// drafts (`claims`), there is no initiating turn to bind a claim to.
async fn submit_owner_draft(
    state: &AppState,
    chat: &ChatAuthority,
    agent: &crate::models::assistant_agent::AssistantAgent,
    body: GeneratedProposal,
    reader: &impl super::agent_skill_service::OrnnReader,
    validator: &impl super::agent_skill_service::OrnnReader,
    seed: impl FnOnce(&str) -> AppResult<String>,
) -> AppResult<Value> {
    let agent = agent.clone();
    let encoded = validate_body(&body)?;
    validation::ornn_rules(&body)?;
    review::validate_proposal_base(&state.db, &agent, &body, ProposalSource::Authored).await?;
    ornn_preflight(validator, &body).await?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(state.audit_chain_hmac_key.as_slice())
        .expect("HMAC key");
    mac.update(b"nyxid-authored-skill-v1\0");
    mac.update(agent.id.as_bytes());
    mac.update(&encoded);
    let fingerprint = hex::encode(mac.finalize().into_bytes());
    let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, seed(&fingerprint)?.as_bytes()).to_string();
    let collection = state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
    let mut inserted = false;
    if collection.find_one(doc! {"_id":&id}).await?.is_none() {
        if collection
            .count_documents(doc! {"agent_id":&agent.id,"status":"pending"})
            .await?
            >= 16
        {
            return Err(AppError::Conflict(
                "Review pending skill drafts before creating more".into(),
            ));
        }
        let now = Utc::now();
        let row = AssistantAgentLearningProposal {
            source: ProposalSource::Authored,
            id: id.clone(),
            agent_id: agent.id.clone(),
            owner_id: agent.user_id.clone(),
            run_id: String::new(),
            status: "pending".into(),
            revision: 0,
            config_revision: 0,
            agent_skills_revision: agent.skills_revision,
            fingerprint,
            input_digest: String::new(),
            model_contract: "authored-skill-v1".into(),
            publication: None,
            failure_code: None,
            evidence: Vec::new(),
            body_encrypted: state.encryption_keys.encrypt(&encoded).await?,
            body_bytes: encoded.len() as i64,
            created_at: now,
            updated_at: now,
        };
        inserted = state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .update_one(
                doc! {"_id":&id},
                doc! {"$setOnInsert": bson::to_document(&row).map_err(|_| AppError::Internal("Skill proposal encoding failed".into()))?},
            )
            .upsert(true)
            .await?
            .upserted_id
            .is_some();
    }
    let binding = review::approval_binding_typed(
        state,
        &chat.user_id,
        &agent.id,
        &id,
        0,
        agent.skills_revision,
        reader,
    )
    .await;
    let binding = match binding {
        Ok(binding) => binding,
        Err(failure) => {
            if inserted {
                collection
                    .delete_one(doc! {"_id":&id,"status":"pending","revision":0,"publication":bson::Bson::Null})
                    .await?;
            }
            if body.base_skill.is_some()
                && matches!(
                    failure.code,
                    Some(
                        review::PublicationFailureCode::OrnnUnavailable
                            | review::PublicationFailureCode::NyxidRefused
                    )
                )
            {
                return Err(AppError::Conflict("source_skill_unavailable: NyxID could not read the source skill from Ornn. Approve any pending Ornn request, then draft again.".into()));
            }
            return Err(failure.error);
        }
    };
    if !chat.guest
        && acks::effective_access_mode(&state.db, chat).await?
            == crate::models::assistant_settings::AccessMode::Autopilot
    {
        return publish_automatically(state, chat, &agent, &id, &binding, reader).await;
    }
    let card = acks::request(
        &state.db,
        chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some(review::TOOL),
            arguments: Some(&binding),
            summary: CARD_SUMMARY,
            platform: false,
        },
    )
    .await?;
    Ok(draft_result(state, &card, &id, &agent.id))
}

/// The automatic path shares the reviewed publication saga, then writes one
/// non-consumable receipt with a link to the persisted proposal.
pub async fn publish_automatically(
    state: &AppState,
    chat: &ChatAuthority,
    agent: &crate::models::assistant_agent::AssistantAgent,
    id: &str,
    binding: &Value,
    reader: &impl super::agent_skill_service::OrnnReader,
) -> AppResult<Value> {
    let proposal = state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":id,"agent_id":&agent.id,"owner_id":&agent.user_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Skill proposal not found".into()))?;
    let name = proposal
        .publication
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("authored skill")
        .to_owned();
    let mut result = Box::pin(review::approve_automatically(
        state,
        chat,
        &agent.id,
        id,
        binding,
        &super::agent_skill_service::OwnerPublicationReader(reader),
    ))
    .await?;
    let url = format!(
        "{}/assistant?agent={}&proposal={}",
        state.config.frontend_url.trim_end_matches('/'),
        agent.id,
        id
    );
    let summary = format!(
        "Skill {} {} published and attached to {}",
        name,
        result["version"].as_str().unwrap_or_default(),
        agent.name
    );
    acks::record_auto_allowed_with_link(
        &state.db,
        chat,
        review::TOOL,
        binding,
        &acks::AutoAllowed {
            class: acks::OwnerActionClass::Sensitive,
            mode: crate::models::assistant_settings::AccessMode::Autopilot,
            reason: "access_mode",
        },
        &summary,
        Some(&url),
    )
    .await;
    result["proposal_id"] = id.into();
    result["agent_id"] = agent.id.clone().into();
    result["proposal_url"] = url.into();
    result["instructions"] = format!(
        "Publication and attachment completed. Do not publish or attach again. {}",
        acks::DECISION_NARRATION
    )
    .into();
    Ok(result)
}

const CARD_SUMMARY: &str = "Review the complete skill draft, publish it privately to Ornn and attach its exact version to this agent.";

/// Whether a proposal of this agent is an authored draft.
pub async fn is_authored(db: &Database, agent_id: &str, proposal_id: &str) -> AppResult<bool> {
    Ok(db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":proposal_id,"agent_id":agent_id,"source":"authored"})
        .await?
        .is_some())
}

/// The live chat of a draft's newest review card, if that chat may still
/// author skills.
pub async fn latest_card_chat(
    db: &Database,
    actor: &str,
    proposal_id: &str,
) -> AppResult<Option<ChatAuthority>> {
    let Some(card) = db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .find_one(doc! {"user_id":actor,"tool_name":review::TOOL,"authored_skill.proposal_id":proposal_id})
        .sort(doc! {"created_at":-1})
        .await?
    else {
        return Ok(None);
    };
    match card_authority(db, actor, &card).await {
        Ok(chat) => Ok(Some(chat)),
        Err(error) if error.is_forbidden() || matches!(error, AppError::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// An owner edit changes the draft its card was bound to, so that card can no
/// longer publish (a renewal only repeats the same binding). Raise a fresh
/// card for the saved revision in `chat`.
pub async fn raise_review_card(
    state: &AppState,
    chat: &ChatAuthority,
    agent_id: &str,
    proposal_id: &str,
    reader: &impl super::agent_skill_service::OrnnReader,
) -> AppResult<Value> {
    require_author(&state.db, chat).await?;
    let agent = team::maintained_agent(&state.db, &chat.user_id, agent_id).await?;
    let row = state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":proposal_id,"agent_id":&agent.id,"source":"authored"})
        .await?
        .ok_or_else(|| AppError::NotFound("Learning proposal not found".into()))?;
    let binding = review::approval_binding_typed(
        state,
        &chat.user_id,
        &agent.id,
        proposal_id,
        row.revision,
        agent.skills_revision,
        reader,
    )
    .await
    .map_err(|failure| failure.error)?;
    if !chat.guest
        && acks::effective_access_mode(&state.db, chat).await?
            == crate::models::assistant_settings::AccessMode::Autopilot
    {
        return publish_automatically(state, chat, &agent, proposal_id, &binding, reader).await;
    }
    let card = acks::request(
        &state.db,
        chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some(review::TOOL),
            arguments: Some(&binding),
            summary: CARD_SUMMARY,
            platform: false,
        },
    )
    .await?;
    Ok(draft_result(state, &card, proposal_id, &agent.id))
}

#[cfg(test)]
#[path = "assistant_skill_authoring_tests.rs"]
mod tests;
