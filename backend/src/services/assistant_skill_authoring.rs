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
    models::assistant_agent_learning::{
        AssistantAgentLearningProposal, PROPOSALS_COLLECTION_NAME, ProposalSource,
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
pub const GUIDANCE: &str = "\nAgents: description defines role/scope; persona defines tone/style; Ornn skills hold repeatable procedures, checklists, domain references, output templates and multi-step workflows. For uncovered procedures, use nyxid__draft_agent_skill only after search_agent_skills, preview_agent_skill and proposing suitable existing skills with set_agent_skills. Drafts need one owner review card; NyxID packages, publishes and pins. Never package or publish skills through Ornn Playground, sandboxes, machines or raw Ornn upload APIs. Never include secrets. End your turn after drafting; the owner card publishes without a tool retry.";

/// No membership reads on the default-off or person-only rollout path.
pub async fn enabled(db: &Database, actor: &str) -> AppResult<bool> {
    match feature_flag_service::flag_enabled_people(db, learning::FLAG_KEY).await? {
        Some(people) => Ok(people.iter().any(|person| person == actor)),
        None => feature_flag_service::personal_flag_enabled(db, actor, learning::FLAG_KEY).await,
    }
}

pub async fn require_author(db: &Database, chat: &ChatAuthority) -> AppResult<()> {
    if chat.guest || !chat.is_orchestrator() {
        return Err(AppError::Forbidden("Only the owner's NyxBot may draft agent skills; specialists can use request_agent_skills".into()));
    }
    if !enabled(db, &chat.user_id).await? {
        return Err(AppError::Forbidden(
            "Agent skill authoring is not enabled".into(),
        ));
    }
    Ok(())
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

fn invalid() -> AppError {
    AppError::ValidationError("Invalid skill draft: use a lowercase hyphenated name (1–64), a short description and bounded .md/.txt files without credentials".into())
}

pub(crate) fn validate_body(body: &GeneratedProposal) -> AppResult<()> {
    // Ornn/Agent Skills slug contract; remote format validation remains authoritative.
    if body.name.is_empty()
        || body.name.len() > 64
        || !body
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || body.name.starts_with('-')
        || body.name.ends_with('-')
        || body.name.contains("--")
        || body.description.trim().is_empty()
        || body
            .skill_md
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        || [&body.name, &body.description, &body.skill_md]
            .into_iter()
            .any(|text| team::looks_secret(text))
        || body
            .files
            .iter()
            .any(|file| team::looks_secret(&file.path) || team::looks_secret(&file.content))
    {
        return Err(invalid());
    }
    learning::validate_generated(&serde_json::to_string(body).map_err(|_| invalid())?)?
        .ok_or_else(invalid)?;
    Ok(())
}

pub async fn create(state: &AppState, chat: &ChatAuthority, input: DraftInput) -> AppResult<Value> {
    require_author(&state.db, chat).await?;
    let agent = team::maintained_agent(&state.db, &chat.user_id, &input.agent).await?;
    if agent.destroyed_at.is_some() {
        return Err(AppError::NotFound("Agent not found".into()));
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
                .ok_or_else(invalid)?;
            if input.name != pin.name {
                return Err(invalid());
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
    validate_body(&body)?;
    review::validate_proposal_base(&state.db, &agent, &body, ProposalSource::Authored).await?;
    let encoded =
        learning::validate_generated(&serde_json::to_string(&body).map_err(|_| invalid())?)?
            .ok_or_else(invalid)?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(state.audit_chain_hmac_key.as_slice())
        .expect("HMAC key");
    mac.update(b"nyxid-authored-skill-v1\0");
    mac.update(agent.id.as_bytes());
    mac.update(&encoded);
    let fingerprint = hex::encode(mac.finalize().into_bytes());
    // Tool retries within the same turn reuse the encrypted proposal and card.
    let turn = chat
        .turn_id
        .as_deref()
        .ok_or_else(|| AppError::Forbidden("Draft from an active owner turn".into()))?;
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("skill-draft:{}:{turn}:{fingerprint}", chat.conversation_id).as_bytes(),
    )
    .to_string();
    let collection = state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
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
        state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .update_one(
                doc! {"_id":&id},
                doc! {"$setOnInsert": bson::to_document(&row).map_err(|_| invalid())?},
            )
            .upsert(true)
            .await?;
    }
    let binding = review::approval_binding(
        state,
        &chat.user_id,
        &agent.id,
        &id,
        0,
        agent.skills_revision,
    )
    .await?;
    let card = acks::request(&state.db, chat, acks::Request {
        kind: "action", service: None, tool: Some(review::TOOL), arguments: Some(&binding),
        summary: "Review the complete skill draft, publish it privately to Ornn and attach its exact version to this agent.", platform: false,
    }).await?;
    let mut result = acks::refusal(&card);
    result["proposal_id"] = id.into();
    result["agent_id"] = agent.id.into();
    result["source"] = "authored".into();
    result["instructions"] = "End your turn. The owner reviews all files on this card; NyxID publishes and pins after approval without a tool retry. Never confirm publication yourself.".into();
    Ok(result)
}

#[cfg(test)]
#[path = "assistant_skill_authoring_tests.rs"]
mod tests;
