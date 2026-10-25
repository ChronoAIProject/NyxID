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
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize)]
pub struct MachineOption {
    node_id: String,
    name: String,
    revision: i64,
    assignment_revision: i64,
    membership: bool,
    capability_edit_enabled: bool,
    context_setup_enabled: bool,
    context_policy_version: &'static str,
    capability_edit_reason: Option<String>,
    context_setup_reason: Option<String>,
    authorization_applied: bool,
    machine_configuration: Option<crate::models::remote_machine::SafeMachineConfigurationView>,
    environment_applied: bool,
    availability_reason: Option<String>,
    protocol_v2: bool,
    can_edit: bool,
    capabilities: nyxid_machine::authority::Capabilities,
    ceiling: nyxid_machine::authority::Capabilities,
    legacy: bool,
    mode: String,
    selection_required: bool,
    selection_reason: Option<String>,
    separation_choice: Option<nyxid_machine::context::Choice>,
    execution_note: &'static str,
    separated_setup_note: Option<&'static str>,
    separated: Option<nyxid_machine::context::Support>,
    saved_login_ids: Option<Vec<String>>,
    revocation_pending: bool,
    delivery_pending: bool,
    quarantine_pending: bool,
    execution_revocation_pending: bool,
    revocation_initialization_pending: bool,
    revocation_expires_at: Option<String>,
    revocation_diagnostic: Option<&'static str>,
}
pub async fn options(
    state: &AppState,
    actor: &str,
    agent_id: &str,
) -> AppResult<Vec<MachineOption>> {
    let agent = team::maintained_agent(&state.db, actor, agent_id).await?;
    let policy = Box::pin(access::policy(&state.db, &agent.id)).await?;
    let contexts = crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        actor,
        access::CONTEXT_FLAG,
    )
    .await?;
    let editing = crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        actor,
        access::FLAG,
    )
    .await?;
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
    let node_ids = nodes.iter().map(|node| node.id.clone()).collect::<Vec<_>>();
    let pending = access::revocation_status(&state.db, &agent.id, &node_ids).await?;
    let hosted: Vec<crate::models::remote_machine::ManagedMachine> = state
        .db
        .collection(crate::models::remote_machine::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":&node_ids}})
        .await?
        .try_collect()
        .await?;
    let hosted = hosted
        .into_iter()
        .map(|machine| (machine.node_id.clone(), machine))
        .collect::<std::collections::HashMap<_, _>>();
    Ok(nodes
        .into_iter()
        .filter_map(|node| {
            let profile = node.machine.clone()?;
            let membership = agent.machine_node_ids.contains(&node.id)
                || (agent.is_nyxbot()
                    && !hosted.contains_key(&node.id)
                    && policy.assignments.contains_key(&node.id));
            let authorization_applied =
                membership && profile.authority_v2() && !pending.contains_key(&node.id);
            let machine_configuration = hosted.get(&node.id).map(|machine| {
                crate::services::remote_machine_configuration::configuration_view(
                    machine,
                    Some(&node),
                )
            });
            let can_edit = writable.contains(&node.user_id);

            let separated = access::separated_support(&profile);
            let assignment = policy
                .assignments
                .get(&node.id)
                .cloned()
                .unwrap_or_else(|| access::unresolved_assignment(policy.revision));
            let selection_required =
                assignment.mode == crate::models::machine_access::SELECTION_REQUIRED;
            let selection_reason = selection_required.then(|| {
                access::selection_message(access::pending_selection_reason(&profile, contexts))
            });
            let availability_reason = if !membership {
                Some("not_assigned")
            } else if selection_required {
                Some("machine_selection_required")
            } else if assignment.capabilities == Default::default() {
                Some("no_tools_granted")
            } else if node.status != crate::models::node::NodeStatus::Online {
                Some("machine_offline")
            } else if hosted.get(&node.id).is_some_and(|machine| {
                machine.resource_state != crate::models::remote_machine::ResourceState::Ready
                    || !machine.writer_admitted
                    || machine.authorization_state
                        != crate::models::remote_machine::AuthorizationState::Active
            }) {
                Some("machine_lifecycle_not_ready")
            } else if hosted.get(&node.id).is_some_and(|machine| {
                !crate::services::remote_machine_runtime::effective_execution_cutoff(machine)
                    .is_some_and(|deadline| deadline > chrono::Utc::now())
            }) {
                Some("machine_runtime_unavailable")
            } else if assignment.mode == "separated" && !separated.available {
                Some("separated_context_unavailable")
            } else if pending.contains_key(&node.id) {
                Some("authority_application_pending")
            } else if machine_configuration
                .as_ref()
                .is_some_and(|c| c.state != "applied")
            {
                Some("machine_configuration_pending")
            } else {
                None
            };
            let environment_applied = authorization_applied
                && !selection_required
                && (assignment.mode != "separated" || separated.available)
                && machine_configuration
                    .as_ref()
                    .is_some_and(|c| c.state == "applied");
            Some(MachineOption {
                assignment_revision: assignment.revision,
                membership,
                capability_edit_enabled: editing,
                context_setup_enabled: contexts,
                context_policy_version: crate::models::machine_access::CONTEXT_POLICY_VERSION,
                capability_edit_reason: if !can_edit {
                    Some("owner_write_required")
                } else if !editing {
                    Some("machine_capability_editing_not_enabled")
                } else if !profile.authority_v2() {
                    Some("machine_authority_upgrade_required")
                } else if selection_required {
                    Some("machine_selection_required")
                } else {
                    None
                }
                .map(str::to_owned),
                context_setup_reason: if !can_edit {
                    Some("owner_write_required".into())
                } else if !profile.authority_v2() {
                    Some("machine_authority_upgrade_required".into())
                } else if !contexts {
                    Some("machine_context_setup_not_enabled".into())
                } else {
                    separated.reason.clone()
                },
                authorization_applied,
                machine_configuration,
                environment_applied,
                availability_reason: availability_reason.map(str::to_owned),
                revocation_pending: pending.get(&node.id).is_some_and(|p| {
                    !profile.authority_timing_v1
                        || p.get_bool("execution_pending").unwrap_or(true)
                        || p.get_bool("quarantine_pending").unwrap_or(false)
                }),
                delivery_pending: pending.contains_key(&node.id),
                quarantine_pending: pending
                    .get(&node.id)
                    .is_some_and(|p| p.get_bool("quarantine_pending").unwrap_or(false)),
                execution_revocation_pending: pending.get(&node.id).is_some_and(|p| {
                    !profile.authority_timing_v1 || p.get_bool("execution_pending").unwrap_or(true)
                }),
                revocation_initialization_pending: pending
                    .get(&node.id)
                    .is_some_and(|p| p.get_bool("initialization_pending").unwrap_or(true)),
                revocation_expires_at: (profile.authority_timing_v1
                    && pending.get(&node.id).is_some_and(|p| {
                        !p.get_bool("invalid_deadline").unwrap_or(true)
                            && !p.get_bool("initialization_pending").unwrap_or(true)
                    }))
                .then(|| {
                    pending
                        .get(&node.id)
                        .and_then(|p| p.get_datetime("expires_at").ok())
                        .map(|d| d.to_chrono().to_rfc3339())
                })
                .flatten(),
                revocation_diagnostic: pending.get(&node.id).and_then(|p| {
                    if p.get_bool("invalid_deadline").unwrap_or(true) {
                        Some("revocation_deadline_invalid")
                    } else if !profile.authority_timing_v1 {
                        Some("authority_timing_upgrade_required")
                    } else {
                        None
                    }
                }),
                node_id: node.id,
                name: node.name,
                revision: policy.revision,
                protocol_v2: profile.authority_v2(),
                can_edit: writable.contains(&node.user_id),
                capabilities: assignment.capabilities,
                ceiling: nyxid_machine::authority::Capabilities::legacy(&profile),
                legacy: assignment.legacy,
                execution_note: crate::services::machine_tools::execution_note(
                    &assignment.mode,
                    separated.available,
                ),
                separated_setup_note: (!separated.available)
                    .then_some(crate::services::machine_tools::SEPARATED_SETUP_NOTE),
                selection_required,
                selection_reason,
                separation_choice: profile.separation_choice,
                mode: assignment.mode,
                separated: Some(separated),
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
    if selection.mode.is_some() {
        return Err(AppError::ValidationError(
            "Request a machine context change in the assistant and approve its owner action card"
                .into(),
        ));
    }
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRequestBody {
    pub mode: nyxid_machine::context::Mode,
    pub policy_version: String,
    pub acknowledge_separation: bool,
}

#[derive(Serialize)]
pub struct ContextRequestResponse {
    pub status: &'static str,
    pub acknowledgement_id: String,
    pub conversation_id: String,
    pub message: &'static str,
}

/// NyxBot card entry point reuses the identical persisted typed action.
pub async fn request_context(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent_id, node_id)): Path<(String, String)>,
    Json(body): Json<ContextRequestBody>,
) -> AppResult<Json<ContextRequestResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let action = crate::services::machine_context_review_service::prepare_action(
        &state.db,
        &actor,
        &agent_id,
        &node_id,
        body.mode,
        &body.policy_version,
        body.acknowledge_separation,
    )
    .await?;
    let bot = Box::pin(team::ensure_nyxbot(&state.db, &actor)).await?;
    let home = Box::pin(team::home_thread(&state.db, &state.encryption_keys, &bot)).await?;
    let chat = crate::services::assistant_acknowledgement_service::for_key(
        &state.db,
        &actor,
        Some(&home.credential_api_key_id),
    )
    .await?
    .ok_or(AppError::MachineBrowserUnavailable)?;
    let summary = if body.mode == nyxid_machine::context::Mode::SharedLegacy {
        format!(
            "Approve shared legacy mode. Tool and saved-login scope are fixed in the owner review. {}",
            nyxid_machine::context::SHARED_LEGACY_WARNING
        )
    } else {
        "Separate workspace and browser. Contexts share the host kernel. Tool and saved-login scope remain fixed; shared files and cookies are not copied.".into()
    };
    let row = crate::services::assistant_acknowledgement_service::request_machine_context(
        &state.db, &chat, action, &summary,
    )
    .await?;
    Ok(Json(ContextRequestResponse {
        status: "pending",
        acknowledgement_id: row.id,
        conversation_id: row.conversation_id,
        message: "Owner approval requested in the NyxBot Assistant thread.",
    }))
}

#[derive(Serialize)]
pub struct MachineConfigurationTargetResponse {
    revision: u64,
    digest: String,
    runtime_generation: i64,
    credential_epoch: i64,
}

#[derive(Serialize)]
pub struct ContextReviewResponse {
    review_id: String,
    agent_id: String,
    node_id: String,
    status: String,
    mode: String,
    policy_version: String,
    expected_revision: i64,
    capabilities: nyxid_machine::authority::Capabilities,
    saved_login_ids: Option<Vec<String>>,
    previous_mode: String,
    capabilities_added: Vec<String>,
    capabilities_removed: Vec<String>,
    saved_login_ids_added: Vec<String>,
    saved_login_ids_removed: Vec<String>,
    warning: &'static str,
    storage_note: &'static str,
    expires_at: String,
    applicable: bool,
    applicability_reason: Option<&'static str>,
    committed_revision: Option<i64>,
    error_code: Option<String>,
    machine_configuration_target: Option<MachineConfigurationTargetResponse>,
    authorization_applied: bool,
    machine_configuration: Option<crate::models::remote_machine::SafeMachineConfigurationView>,
    environment_applied: bool,
}
async fn review_response(
    state: &AppState,
    actor: &str,
    row: crate::models::machine_access::ContextReview,
) -> AppResult<ContextReviewResponse> {
    let applicable = crate::services::machine_context_review_service::validate_action(
        &state.db,
        actor,
        &row.action,
    )
    .await
    .is_ok();
    let machine = Box::pin(options(state, actor, &row.action.agent_id))
        .await?
        .into_iter()
        .find(|m| m.node_id == row.action.node_id);
    let current = machine.as_ref();
    let authorization_applied = row.committed_revision.is_some_and(|revision| {
        current.is_some_and(|m| m.assignment_revision == revision && m.authorization_applied)
    });
    let environment_applied =
        authorization_applied && current.is_some_and(|m| m.environment_applied);
    let old = &row.action.snapshot.previous_assignment;
    let new = &row.action.selection;
    let before = old.saved_login_ids.as_deref().unwrap_or_default();
    let after = new.saved_login_ids.as_deref().unwrap_or_default();
    Ok(ContextReviewResponse {
        review_id: row.id,
        agent_id: row.action.agent_id,
        node_id: row.action.node_id,
        status: row.status.clone(),
        mode: new.mode.clone().unwrap_or_default(),
        policy_version: row.action.snapshot.policy_version,
        expected_revision: new.expected_revision,
        capabilities: new.capabilities,
        saved_login_ids: new.saved_login_ids.clone(),
        previous_mode: old.mode.clone(),
        capabilities_added: vec![],
        capabilities_removed: vec![],
        saved_login_ids_added: after
            .iter()
            .filter(|id| !before.contains(id))
            .cloned()
            .collect(),
        saved_login_ids_removed: before
            .iter()
            .filter(|id| !after.contains(id))
            .cloned()
            .collect(),
        warning: nyxid_machine::context::SHARED_LEGACY_WARNING,
        storage_note: "Mode changes do not move shared files or copy cookies. Separated workspaces are retained; HOME, browser profiles, downloads and tmp are temporary. Browser work waits for required profile cleanup.",
        expires_at: row.expires_at.to_rfc3339(),
        applicable,
        applicability_reason: (!applicable && row.status != "committed")
            .then_some("machine_context_review_stale"),
        committed_revision: row.committed_revision,
        error_code: row.error_code,
        machine_configuration_target: row.action.snapshot.machine_configuration.map(|target| {
            MachineConfigurationTargetResponse {
                revision: target.revision,
                digest: target.digest,
                runtime_generation: target.runtime_generation,
                credential_epoch: target.credential_epoch,
            }
        }),
        authorization_applied,
        machine_configuration: machine.and_then(|m| m.machine_configuration),
        environment_applied,
    })
}
pub async fn prepare_context(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent, node)): Path<(String, String)>,
    Json(body): Json<ContextRequestBody>,
) -> AppResult<Json<ContextReviewResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = crate::services::machine_context_review_service::prepare(
        &state.db,
        &actor,
        &agent,
        &node,
        body.mode,
        &body.policy_version,
        body.acknowledge_separation,
    )
    .await?;
    Ok(Json(review_response(&state, &actor, row).await?))
}
pub async fn review_context(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent, node, review)): Path<(String, String, String)>,
) -> AppResult<Json<ContextReviewResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = crate::services::machine_context_review_service::get(
        &state.db, &actor, &agent, &node, &review,
    )
    .await?;
    Ok(Json(review_response(&state, &actor, row).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitContextBody {}
pub async fn commit_context(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent, node, review)): Path<(String, String, String)>,
    Json(_): Json<CommitContextBody>,
) -> AppResult<Json<ContextReviewResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = crate::services::machine_context_review_service::commit(
        &state.db, &actor, &agent, &node, &review,
    )
    .await?;
    Ok(Json(review_response(&state, &actor, row).await?))
}

/// Apply a graphical owner-card decision through the same fenced transaction
/// as the assistant tool path. The stored typed payload is the authority for
/// this operation; request data is never re-read from the client.
pub async fn apply_human_context_action(
    state: &AppState,
    actor: &str,
    action: &crate::models::machine_access::HumanContextAction,
) -> AppResult<()> {
    if !action
        .selection
        .mode
        .as_deref()
        .is_some_and(|mode| matches!(mode, "separated" | "shared_legacy"))
    {
        return Err(AppError::ValidationError(
            "An explicit machine separation choice is required".into(),
        ));
    }
    let policy = Box::pin(access::configure_reviewed(&state.db, actor, action, None)).await?;
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(actor.to_owned()),
        "machine_context_mode_changed".into(),
        Some(serde_json::json!({
            "agent_id": action.agent_id,
            "node_id": action.node_id,
            "mode": action.selection.mode,
            "revision": policy.revision,
            "source": "human_action_card"
        })),
        None,
        None,
        None,
        None,
    );
    Ok(())
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
    if selection.mode.as_deref() == Some("shared_legacy") {
        return Ok((
            json!({"status":"selection_required", "message":"Only the owner can explicitly approve shared legacy access in Assistant → Machines after reviewing the warning.", "settings_path":crate::services::assistant_links::AssistantPage::MachineSettings {node}.path(), "warning":nyxid_machine::context::SHARED_LEGACY_WARNING}),
            true,
        ));
    }
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
    if selection.mode.is_some()
        && !crate::services::feature_flag_service::personal_flag_enabled(
            &state.db,
            &chat.user_id,
            access::CONTEXT_FLAG,
        )
        .await?
    {
        return Err(AppError::ValidationError(
            "Machine context setup is not enabled; existing separation still applies".into(),
        ));
    }
    if selection
        .mode
        .as_deref()
        .is_some_and(|mode| !matches!(mode, "separated" | "automatic" | "require_separated"))
    {
        return Err(AppError::ValidationError("Choose automatic or require_separated; shared legacy requires the human settings action card".into()));
    }
    let recorded_choice_usable = machine.separation_choice
        == Some(nyxid_machine::context::Choice::SharedLegacy)
        || (machine.separation_choice.is_some()
            && machine
                .separated
                .as_ref()
                .is_some_and(|support| support.available)
            && crate::services::feature_flag_service::personal_flag_enabled(
                &state.db,
                &chat.user_id,
                access::CONTEXT_FLAG,
            )
            .await?);
    if selection.mode.is_none() && machine.selection_required && !recorded_choice_usable {
        return Ok((
            json!({"status":"selection_required", "message":machine.selection_reason, "settings_path":crate::services::assistant_links::AssistantPage::MachineSettings {node}.path(), "warning":nyxid_machine::context::SHARED_LEGACY_WARNING}),
            true,
        ));
    }
    if selection.mode.is_some() && machine.separated.as_ref().is_none_or(|s| !s.available) {
        return Err(AppError::ValidationError(access::selection_message(
            machine
                .separated
                .as_ref()
                .and_then(|support| support.reason.as_deref())
                .unwrap_or("separated_capabilities_not_reported"),
        )));
    }
    let old = policy.assignments.get(node).cloned().unwrap_or_default();
    let changes_mode = selection
        .mode
        .as_ref()
        .is_some_and(|mode| *mode != old.mode);
    let widens = changes_mode
        || !selection.capabilities.subset_of(old.capabilities)
        || match (&old.saved_login_ids, &selection.saved_login_ids) {
            (Some(old), Some(new)) => new.iter().any(|id| !old.contains(id)),
            (Some(_), None) => true,
            _ => false,
        };
    let mut auto_allowed = None;
    let mut receipt_summary = String::new();
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
            let mode_note = if changes_mode {
                " Separate workspace and browser uses fresh profiles; full isolation requires a separate machine container or VM. Changing mode stops current work and does not copy files or cookies."
            } else {
                ""
            };
            receipt_summary = format!(
                "Change {} on machine {}: shell={}, files={}, browser={}, computer={}, developer browser={}; {}. Current work for this agent on this machine will stop.{mode_note}",
                agent.name,
                machine.name,
                selection.capabilities.shell,
                selection.capabilities.files,
                selection.capabilities.browser,
                selection.capabilities.computer,
                selection.capabilities.developer_browser,
                login_scope
            );
            if chat.is_orchestrator() {
                let gate = acks::confirmation_needed(
                    &state.db,
                    chat,
                    acks::ActionFacts::of(acks::OwnerActionClass::Sensitive),
                    false,
                )
                .await?;
                auto_allowed = gate.auto_allowed();
                if let Some(need) = gate.ask()
                    && let Some(refusal) =
                        acks::confirm_action(&state.db, chat, tool, args, &need, &receipt_summary)
                            .await?
                {
                    return Ok((refusal, true));
                }
            } else {
                if !chat.involved() {
                    acks::audit_missing_access(
                        &state.db,
                        chat,
                        Some(node),
                        crate::services::assistant_access_policy::AccessArea::MachineCapabilities
                            .scope(),
                    )
                    .await;
                    return Ok((
                        crate::services::assistant_access_policy::missing_access(
                            crate::services::assistant_access_policy::AccessArea::MachineCapabilities,
                            Some(&machine.name),
                            &agent.id,
                            None,
                        ),
                        true,
                    ));
                }
                let routed =
                    crate::services::assistant_settings_service::get(&state.db, &chat.user_id)
                        .await
                        .map(|s| {
                            chat.access_request_allowed_with_mode(s.specialist_access_requests)
                        })
                        .unwrap_or(false);
                if !routed {
                    acks::audit_access_request_denied(
                        &state.db,
                        chat,
                        Some(node),
                        "machine_capabilities",
                    )
                    .await;
                    return Ok((acks::access_request_denied(), true));
                }
                if acks::effective_access_mode(&state.db, chat).await?
                    == crate::models::assistant_settings::AccessMode::Autopilot
                {
                    let (request, created) = acks::request_tracked(
                        &state.db, chat, acks::Request {
                            kind: "machine", service: Some((node, node, &machine.name)),
                            tool: Some(tool), arguments: Some(args),
                            summary: &format!("{receipt_summary} Desired selection: {}. After this request is decided, the current machine policy is inspected and these capabilities are applied with nyxid__machine_capabilities.", args["selection"]),
                            platform: false,
                        }).await?;
                    if created {
                        super::assistant_team::permission_requested(state, chat, &request).await;
                    }
                    return Ok((acks::refusal(&request), true));
                }
                let card = acks::request(
                    &state.db,
                    chat,
                    acks::Request {
                        kind: "action",
                        service: None,
                        tool: Some(tool),
                        arguments: Some(args),
                        summary: &receipt_summary,
                        platform: false,
                    },
                )
                .await?;
                return Ok((acks::refusal(&card), true));
            }
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
    if let Some(auto) = auto_allowed {
        acks::record_auto_allowed(
            &state.db,
            chat,
            "nyxid__machine_capabilities",
            args,
            &auto,
            &receipt_summary,
        )
        .await;
    }
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
        json!({"revision":policy.revision,"machine":node,"revocation":"Earlier commands stop after machine acknowledgement or the reported recovery time. Reconnect and refresh machine status; browser cleanup requires acknowledgement"}),
        false,
    ))
}
