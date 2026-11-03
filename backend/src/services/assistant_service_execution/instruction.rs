use super::*;
use crate::models::user::{User, UserType};

pub(crate) async fn active(db: &Database, actor: &str) -> AppResult<bool> {
    super::super::feature_flag_service::personal_flag_enabled(
        db,
        actor,
        super::super::feature_flag_service::ORG_INSTRUCTION_FLAG_KEY,
    )
    .await
}

/// Turn guidance for the organization-instruction rule. Added to the stable
/// instructions only when the rule applies to this owner and agent.
pub(crate) const PROMPT_GUIDANCE: &str = "\nOrganization connections need this turn's owner selection (/service) or an approved card in every access mode, including Full access. Ask before using one; prefer offering nyx__connect_service for the owner's own account.\n";

/// Whether the organization-instruction rule applies to this owner's agent
/// (personal agents only; organization-owned specialists are excluded).
pub(crate) async fn prompt_applies(
    db: &Database,
    owner: &str,
    agent: Option<&crate::models::assistant_agent::AssistantAgent>,
) -> AppResult<bool> {
    if !active(db, owner).await? {
        return Ok(false);
    }
    let Some(agent) = agent else {
        return Ok(true);
    };
    Ok(db
        .collection::<User>("users")
        .find_one(doc! {"_id":&agent.user_id,"is_active":true})
        .await?
        .is_none_or(|owner| owner.user_type != UserType::Org))
}

/// Agent ownership is independent of the shared-personal access branch.
pub(crate) async fn personal_agent(db: &Database, chat: &acks::ChatAuthority) -> AppResult<bool> {
    if chat.agent_id.is_empty() {
        return Ok(true);
    }
    let agent = db
        .collection::<crate::models::assistant_agent::AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":&chat.agent_id})
        .await?;
    let owner = agent
        .as_ref()
        .map(|a| a.user_id.as_str())
        .unwrap_or(&chat.user_id);
    Ok(db
        .collection::<User>("users")
        .find_one(doc! {"_id":owner,"is_active":true})
        .await?
        .is_none_or(|owner| owner.user_type != UserType::Org))
}

pub(crate) async fn covered(
    db: &Database,
    chat: &acks::ChatAuthority,
    service: &UserService,
) -> AppResult<bool> {
    // Cheapest checks first: the actor's own rows and platform/auto rows never
    // need instruction, so ordinary personal calls add no database reads.
    if service.user_id == chat.user_id
        || service.source.as_deref() == Some(crate::models::user_service::AUTO_PROVISION_SOURCE)
        || service.credential_binding.as_deref() == Some("platform")
    {
        return Ok(false);
    }
    let org_owned = db
        .collection::<User>("users")
        .find_one(doc! {"_id":&service.user_id,"is_active":true})
        .await?
        .is_some_and(|owner| owner.user_type == UserType::Org);
    Ok(org_owned && active(db, &chat.user_id).await? && personal_agent(db, chat).await?)
}

/// This is positive, server-issued evidence for one exact connection in one live
/// turn. Legacy selections and card-derived selections deliberately fail it.
pub(crate) async fn owner_instruction(
    db: &Database,
    chat: &acks::ChatAuthority,
    service: &UserService,
) -> AppResult<bool> {
    let Some(row) = selection::selected(db, chat).await? else {
        return Ok(false);
    };
    Ok(row.get_i32("owner_instruction_version") == Ok(1)
        && row.get_str("selection_source") == Ok("owner")
        && row.get_str("connection_id") == Ok(service.id.as_str())
        && selection_matches(&row, service, ""))
}
