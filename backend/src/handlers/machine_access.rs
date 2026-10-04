//! Human policy editor and owner-reviewed native capability changes.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::machine_access::Selection,
    mw::auth::AuthUser,
    services::{assistant_team_service as team, machine_access_service as access},
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Serialize;

#[derive(Serialize)]
pub struct MachineOption {
    node_id: String,
    name: String,
    revision: i64,
    protocol_v2: bool,
    can_edit: bool,
    capabilities: nyxid_machine::authority::Capabilities,
    ceiling: nyxid_machine::authority::Capabilities,
    legacy: bool,
    saved_login_ids: Option<Vec<String>>,
    revocation_pending: bool,
}
pub async fn options(
    state: &AppState,
    actor: &str,
    agent_id: &str,
) -> AppResult<Vec<MachineOption>> {
    let agent = team::maintained_agent(&state.db, actor, agent_id).await?;
    let policy = Box::pin(access::policy(&state.db, &agent.id)).await?;
    let writable = crate::services::machine_service::usable_owners(&state.db, actor).await?;
    let owners = if agent.user_id != actor {
        vec![agent.user_id.clone()]
    } else {
        writable.clone()
    };
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    let nodes:Vec<crate::models::node::Node>=state.db.collection(crate::models::node::COLLECTION_NAME)
        .find(doc!{"user_id":{"$in":owners},"is_active":true,"machine":{"$ne":mongodb::bson::Bson::Null}})
        .sort(doc!{"name":1,"_id":1}).limit(500).await?.try_collect().await?;
    let pending: Vec<mongodb::bson::Document> = state
        .db
        .collection::<mongodb::bson::Document>(crate::models::machine_access::OUTBOX)
        .find(doc! {"agent_id":&agent.id,"pending":true})
        .projection(doc! {"node_id":1})
        .limit(500)
        .await?
        .try_collect()
        .await?;
    Ok(nodes
        .into_iter()
        .filter_map(|node| {
            let profile = node.machine?;
            let assignment = policy
                .assignments
                .get(&node.id)
                .cloned()
                .unwrap_or_default();
            Some(MachineOption {
                revocation_pending: pending
                    .iter()
                    .any(|p| p.get_str("node_id").ok() == Some(node.id.as_str())),
                node_id: node.id,
                name: node.name,
                revision: policy.revision,
                protocol_v2: profile.authority_v2(),
                can_edit: writable.contains(&node.user_id),
                capabilities: assignment.capabilities,
                ceiling: nyxid_machine::authority::Capabilities::legacy(&profile),
                legacy: assignment.legacy,
                saved_login_ids: assignment.saved_login_ids,
            })
        })
        .collect())
}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent): Path<String>,
) -> AppResult<Json<Vec<MachineOption>>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        Box::pin(options(&state, &auth.user_id.to_string(), &agent)).await?,
    ))
}
pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent, node)): Path<(String, String)>,
    Json(selection): Json<Selection>,
) -> AppResult<Json<Vec<MachineOption>>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let revision = Box::pin(access::configure(
        &state.db, &actor, &agent, &node, selection,
    ))
    .await?
    .revision;
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(actor.clone()),
        "machine_capabilities_changed".into(),
        Some(serde_json::json!({"agent_id":agent,"node_id":node,"revision":revision})),
        None,
        None,
        None,
        None,
    );
    Ok(Json(Box::pin(options(&state, &actor, &agent)).await?))
}

pub async fn native(
    state: &AppState,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
    args: &serde_json::Value,
) -> AppResult<(serde_json::Value, bool)> {
    use crate::services::assistant_acknowledgement_service as acks;
    use serde_json::json;
    if chat.guest {
        return Err(AppError::MachineNotAllowed);
    }
    let requested = args["agent"].as_str();
    let agent = if requested.is_none_or(|id| id == chat.agent_id) {
        // The ordinary named-target resolver selects specialists. Self-targeting
        // must also support NyxBot and preserve an org caller's live authority.
        Box::pin(crate::services::org_agent_service::chat_agent(
            &state.db, chat,
        ))
        .await?
    } else {
        super::assistant_team::target_agent(state, &chat.user_id, requested).await?
    };
    if !chat.is_orchestrator() && agent.id != chat.agent_id {
        return Err(AppError::MachineNotAllowed);
    }
    if args["selection"].is_null() {
        return Ok((
            json!({"machines":Box::pin(options(state,&chat.user_id,&agent.id)).await?}),
            false,
        ));
    }
    let node = args["machine"]
        .as_str()
        .ok_or_else(|| AppError::ValidationError("machine ID required".into()))?;
    let selection: Selection = serde_json::from_value(args["selection"].clone())
        .map_err(|_| AppError::ValidationError("Invalid capability selection".into()))?;
    if !crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        &chat.user_id,
        access::FLAG,
    )
    .await?
    {
        return Err(AppError::ValidationError(
            "Machine capability editing is not enabled; existing restrictions still apply".into(),
        ));
    }
    let policy = Box::pin(access::policy(&state.db, &agent.id)).await?;
    if selection.expected_revision != policy.revision {
        return Err(AppError::Conflict("Machine access changed; reload".into()));
    }
    let machine = Box::pin(options(state, &chat.user_id, &agent.id))
        .await?
        .into_iter()
        .find(|option| option.node_id == node)
        .ok_or(AppError::MachineNotAllowed)?;
    if !machine.can_edit {
        return Err(AppError::MachineNotAllowed);
    }
    if !machine.protocol_v2 {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    if !selection.capabilities.valid() || !selection.capabilities.subset_of(machine.ceiling) {
        return Err(AppError::MachineCapabilityDisabled);
    }
    let old = policy.assignments.get(node).cloned().unwrap_or_default();
    let widens = !selection.capabilities.subset_of(old.capabilities)
        || match (&old.saved_login_ids, &selection.saved_login_ids) {
            (Some(old), Some(new)) => new.iter().any(|id| !old.contains(id)),
            (Some(_), None) => true,
            _ => false,
        };
    if widens || !chat.is_orchestrator() {
        let tool = "nyxid__machine_capabilities";
        let approved = if let Some(id) = args["acknowledgement_id"].as_str() {
            acks::consume_action(&state.db, chat, id, tool, args).await?
        } else {
            false
        };
        if !approved {
            let login_scope = selection.saved_login_ids.as_ref().map_or_else(
                || "inherit the agent's saved-login grants".to_owned(),
                |ids| format!("limit saved logins to {} selected", ids.len()),
            );
            let card=acks::request(&state.db,chat,acks::Request{kind:"action",service:None,tool:Some(tool),arguments:Some(args),
                summary:&format!("Change {} on machine {}: shell={}, files={}, browser={}, computer={}, developer browser={}; {}. Current work for this agent on this machine will stop.",agent.name,machine.name,selection.capabilities.shell,selection.capabilities.files,selection.capabilities.browser,selection.capabilities.computer,selection.capabilities.developer_browser,login_scope),platform:false}).await?;
            return Ok((acks::refusal(&card), true));
        }
    }
    let policy = Box::pin(access::configure(
        &state.db,
        &chat.user_id,
        &agent.id,
        node,
        selection,
    ))
    .await?;
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(chat.user_id.clone()),
        "machine_capabilities_changed".into(),
        Some(json!({"agent_id":agent.id,"node_id":node,"revision":policy.revision})),
        None,
        None,
        None,
        None,
    );
    Ok((
        json!({"revision":policy.revision,"machine":node,"revocation":"pending until node acknowledges or authority lease expires (at most 45 seconds)"}),
        false,
    ))
}
