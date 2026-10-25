//! First-party owner mode reviews. The stored typed action is the only input to
//! configure; no browser Selection, consent or runtime report is an authority.
use super::{assistant_team_service as team, machine_access_service as access};
use crate::{
    errors::{AppError, AppResult},
    models::{
        machine_access::{
            self as model, ContextActionSnapshot, ContextMachineConfiguration, ContextReview,
            HumanContextAction, Selection,
        },
        node::{self, Node},
    },
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use nyxid_machine::context::Mode;

pub fn stale() -> AppError {
    AppError::Conflict(
        "machine_context_review_stale: prepare and review the current mode change".into(),
    )
}
fn digest(value: &impl serde::Serialize) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(value)
            .map_err(|_| AppError::Internal("Machine review encoding failed".into()))?,
    )))
}
pub fn profile_digest(profile: &nyxid_machine::MachineProfile) -> AppResult<String> {
    digest(
        &serde_json::json!({"version":profile.version,"runtime_id":profile.runtime_id,
        "authority_versions":profile.authority_versions,"authority_timing_v1":profile.authority_timing_v1,
        "shell":profile.shell,"files":profile.files,"browser":profile.browser_enabled(),"computer":profile.computer,
        "separation_choice":profile.separation_choice,"separated":profile.separated,"evidence":profile.separation_evidence,
        "commands_isolated":profile.commands_isolated,"roots":profile.roots,"managed_configuration":profile.managed_configuration,
        "os":profile.os,"arch":profile.arch,"computer_mode":profile.computer_mode,"cua_version":profile.cua_version,
        "computer_tools":profile.computer_tools,"computer_ready":profile.computer_ready,"computer_permissions":profile.computer_permissions,
        "browser_isolated":profile.browser_isolated,"saved_login_ready":profile.saved_login_ready,"browser_tools":profile.browser_tools}),
    )
}
fn membership_digest(rows: &[Document]) -> AppResult<String> {
    // Ignore shared lock counters and timestamps; bind the live scope and role.
    let mut scopes=rows.iter().map(|r| serde_json::json!({"id":r.get_str("_id").ok(),"org":r.get_str("org_user_id").ok(),
        "role":r.get_str("role").ok(),"scope_source":r.get_str("scope_source").ok(),"allowed_service_ids":r.get("allowed_service_ids")})).collect::<Vec<_>>();
    scopes.sort_by_key(|v| v.to_string());
    digest(&scopes)
}
async fn memberships(db: &Database, actor: &str) -> AppResult<Vec<Document>> {
    let rows: Vec<Document> = db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .find(doc! {"member_user_id":actor,"revoked_at":bson::Bson::Null})
        .limit(257)
        .await?
        .try_collect()
        .await?;
    if rows.len() > 256 {
        return Err(stale());
    }
    Ok(rows)
}
async fn flag_epoch(db: &Database) -> AppResult<i64> {
    let row = db
        .collection::<Document>("platform_settings")
        .find_one(doc! {"_id":model::CONTEXT_REVIEW_FENCE})
        .await?;
    Ok(row.and_then(|r| r.get_i64("epoch").ok()).unwrap_or(0))
}
async fn machine_configuration(
    db: &Database,
    node: &Node,
) -> AppResult<Option<ContextMachineConfiguration>> {
    let Some(machine) =
        super::remote_machine_service::managed_machine_for_node(db, &node.id).await?
    else {
        return Ok(None);
    };
    if super::remote_machine_configuration::configuration_view(&machine, Some(node)).state
        != "applied"
    {
        return Err(AppError::Conflict(
            "machine_configuration_pending: wait for current authenticated runtime evidence".into(),
        ));
    }
    Ok(Some(ContextMachineConfiguration {
        revision: machine.configuration.revision,
        digest: machine.configuration.digest().map_err(|_| stale())?,
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
    }))
}

pub async fn prepare_action(
    db: &Database,
    actor: &str,
    agent_id: &str,
    node_id: &str,
    mode: Mode,
    policy_version: &str,
    acknowledge: bool,
) -> AppResult<HumanContextAction> {
    if policy_version != model::CONTEXT_POLICY_VERSION || !acknowledge {
        return Err(AppError::ValidationError(
            "Current machine context policy and explicit acknowledgement are required".into(),
        ));
    }
    // Ensure the common dependency row exists before any snapshot is captured.
    db.collection::<Document>("platform_settings")
        .update_one(
            doc! {"_id":model::CONTEXT_REVIEW_FENCE},
            doc! {"$setOnInsert":{"epoch":0_i64}},
        )
        .upsert(true)
        .await?;
    let epoch = flag_epoch(db).await?;
    let agent = team::maintained_agent(db, actor, agent_id).await?;
    let node = super::node_service::get_node_by_id(db, node_id)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if !node.is_active
        || !super::org_service::resolve_owner_access(db, actor, &node.user_id)
            .await?
            .can_write()
        || (agent.user_id != actor && node.user_id != agent.user_id)
    {
        return Err(AppError::MachineNotAllowed);
    }
    let profile = node
        .machine
        .as_ref()
        .ok_or(AppError::MachineCapabilityDisabled)?;
    if !profile.authority_v2() {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    let editing =
        super::feature_flag_service::personal_flag_enabled(db, actor, access::FLAG).await?;
    let contexts =
        super::feature_flag_service::personal_flag_enabled(db, actor, access::CONTEXT_FLAG).await?;
    if mode == Mode::Separated && (!contexts || !access::supports_separated(profile)) {
        let support = access::separated_support(profile);
        return Err(AppError::ValidationError(access::selection_message(
            if !contexts {
                "machine_context_setup_not_enabled"
            } else {
                support
                    .reason
                    .as_deref()
                    .unwrap_or("separated_capabilities_not_reported")
            },
        )));
    }
    let policy = Box::pin(access::policy(db, &agent.id)).await?;
    let configuration = machine_configuration(db, &node).await?;
    let old = policy.assignments.get(node_id).cloned().unwrap_or_else(|| {
        let mut assignment = access::unresolved_assignment(policy.revision);
        if configuration.is_some() {
            assignment.saved_login_ids = Some(vec![]);
        }
        assignment
    });
    let action = HumanContextAction {
        agent_id: agent.id,
        node_id: node.id.clone(),
        selection: Selection {
            mode: Some(
                match mode {
                    Mode::Separated => "separated",
                    Mode::SharedLegacy => "shared_legacy",
                }
                .into(),
            ),
            expected_revision: policy.revision,
            capabilities: old.capabilities,
            saved_login_ids: old
                .saved_login_ids
                .clone()
                .or_else(|| (mode == Mode::Separated).then(Vec::new)),
        },
        snapshot: ContextActionSnapshot {
            actor_id: actor.into(),
            agent_owner_id: agent.user_id,
            owner_id: node.user_id,
            previous_assignment: old,
            policy_version: policy_version.into(),
            profile_digest: profile_digest(profile)?,
            capability_edit_enabled: editing,
            context_setup_enabled: contexts,
            flag_epoch: epoch,
            memberships_digest: membership_digest(&memberships(db, actor).await?)?,
            machine_configuration: configuration,
            expires_at: chrono::DateTime::from_timestamp_millis(
                (Utc::now() + chrono::Duration::minutes(10)).timestamp_millis(),
            )
            .ok_or_else(stale)?,
        },
    };
    validate_action(db, actor, &action).await?;
    Ok(action)
}

pub async fn validate_action(
    db: &Database,
    actor: &str,
    action: &HumanContextAction,
) -> AppResult<()> {
    let s = &action.snapshot;
    if actor != s.actor_id
        || s.policy_version != model::CONTEXT_POLICY_VERSION
        || s.expires_at <= Utc::now()
    {
        return Err(stale());
    }
    let agent = team::maintained_agent(db, actor, &action.agent_id).await?;
    if agent.user_id != s.agent_owner_id {
        return Err(stale());
    }
    let node = super::node_service::get_node_by_id(db, &action.node_id)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if !node.is_active
        || node.user_id != s.owner_id
        || !super::org_service::resolve_owner_access(db, actor, &node.user_id)
            .await?
            .can_write()
    {
        return Err(AppError::MachineNotAllowed);
    }
    let profile = node
        .machine
        .as_ref()
        .ok_or(AppError::MachineCapabilityDisabled)?;
    if profile_digest(profile)? != s.profile_digest
        || !profile.authority_v2()
        || flag_epoch(db).await? != s.flag_epoch
        || super::feature_flag_service::personal_flag_enabled(db, actor, access::FLAG).await?
            != s.capability_edit_enabled
        || super::feature_flag_service::personal_flag_enabled(db, actor, access::CONTEXT_FLAG)
            .await?
            != s.context_setup_enabled
        || membership_digest(&memberships(db, actor).await?)? != s.memberships_digest
        || machine_configuration(db, &node).await? != s.machine_configuration
        || Box::pin(access::policy(db, &action.agent_id))
            .await?
            .revision
            != action.selection.expected_revision
    {
        return Err(stale());
    }
    Ok(())
}

/// Called inside configure's transaction before any grant/context/outbox write.
/// The locks share existing account/membership and WSS/lifecycle writer rows.
pub async fn fence_action(
    db: &Database,
    tx: &mut ClientSession,
    actor: &str,
    action: &HumanContextAction,
) -> AppResult<()> {
    let s = &action.snapshot;
    if actor != s.actor_id
        || s.policy_version != model::CONTEXT_POLICY_VERSION
        || s.expires_at <= Utc::now()
    {
        return Err(stale());
    }
    super::org_membership_authority::fence_live_member(db, actor, tx).await?;
    let rows: Vec<Document> = db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .find(doc! {"member_user_id":actor,"revoked_at":bson::Bson::Null})
        .limit(257)
        .session(&mut *tx)
        .await?
        .stream(&mut *tx)
        .try_collect()
        .await?;
    if rows.len() > 256 || membership_digest(&rows)? != s.memberships_digest {
        return Err(stale());
    }
    let mut orgs = rows
        .iter()
        .filter_map(|r| r.get_str("org_user_id").ok())
        .collect::<Vec<_>>();
    orgs.sort();
    orgs.dedup();
    for org in orgs {
        super::org_membership_authority::fence_org(db, org, tx).await?;
    }
    let epoch = db
        .collection::<Document>("platform_settings")
        .update_one(
            doc! {"_id":model::CONTEXT_REVIEW_FENCE,"epoch":s.flag_epoch},
            doc! {"$inc":{"review_commit_fence":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    if epoch.matched_count != 1 {
        return Err(stale());
    }
    let node = db
        .collection::<Node>(node::COLLECTION_NAME)
        .find_one(doc! {"_id":&action.node_id,"is_active":true,"user_id":&s.owner_id})
        .session(&mut *tx)
        .await?
        .ok_or_else(stale)?;
    let profile = node.machine.as_ref().ok_or_else(stale)?;
    if profile_digest(profile)? != s.profile_digest || !profile.authority_v2() {
        return Err(stale());
    }
    db.collection::<Document>(node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&node.id},
            doc! {"$inc":{"machine_context_review_fence":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    if let Some(binding) = s.machine_configuration.as_ref() {
        let machine = db
            .collection::<crate::models::remote_machine::ManagedMachine>(
                crate::models::remote_machine::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&node.id})
            .session(&mut *tx)
            .await?
            .ok_or_else(stale)?;
        if binding.revision != machine.configuration.revision
            || binding.digest != machine.configuration.digest().map_err(|_| stale())?
            || binding.runtime_generation != machine.runtime_generation
            || binding.credential_epoch != machine.credential_epoch
            || super::remote_machine_configuration::configuration_view(&machine, Some(&node)).state
                != "applied"
        {
            return Err(stale());
        }
        db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&node.id,"revision":machine.revision},
                doc! {"$inc":{"machine_context_review_fence":1_i64}},
            )
            .session(&mut *tx)
            .await?;
    }
    // No flag mutation can commit while the common epoch row is held. Account,
    // membership and node locks similarly hold live preflight inputs stable.
    if super::feature_flag_service::personal_flag_enabled(db, actor, access::FLAG).await?
        != s.capability_edit_enabled
        || super::feature_flag_service::personal_flag_enabled(db, actor, access::CONTEXT_FLAG)
            .await?
            != s.context_setup_enabled
    {
        return Err(stale());
    }
    if action.selection.mode.as_deref() == Some("separated")
        && (!s.context_setup_enabled || !access::supports_separated(profile))
    {
        return Err(stale());
    }
    // Locks on active owner/Agent-owner records fence account lifecycle changes.
    let agent = team::maintained_agent(db, actor, &action.agent_id).await?;
    if agent.user_id != s.agent_owner_id {
        return Err(stale());
    }
    let mut owners = vec![agent.user_id.as_str(), s.owner_id.as_str()];
    owners.sort();
    owners.dedup();
    for owner in owners {
        let live = db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":owner,"is_active":true,"platform_personal_deleting":{"$ne":true}},
                doc! {"$inc":{"machine_context_review_fence":1_i64}},
            )
            .session(&mut *tx)
            .await?;
        if live.matched_count != 1 {
            return Err(stale());
        }
    }
    if !super::org_service::resolve_owner_access(db, actor, &s.owner_id)
        .await?
        .can_write()
    {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(())
}

pub async fn prepare(
    db: &Database,
    actor: &str,
    agent: &str,
    node: &str,
    mode: Mode,
    version: &str,
    acknowledge: bool,
) -> AppResult<ContextReview> {
    let action = prepare_action(db, actor, agent, node, mode, version, acknowledge).await?;
    let row = ContextReview {
        id: uuid::Uuid::new_v4().to_string(),
        actor_id: actor.into(),
        expires_at: action.snapshot.expires_at,
        action,
        status: "pending".into(),
        committed_revision: None,
        error_code: None,
        created_at: Utc::now(),
    };
    db.collection::<ContextReview>(model::ContextReview::COLLECTION_NAME)
        .insert_one(&row)
        .await?;
    super::audit_service::log_async(
        db.clone(),
        Some(actor.into()),
        "machine_context_review_prepared".into(),
        Some(
            serde_json::json!({"review_id":row.id,"agent_id":agent,"node_id":node,"mode":row.action.selection.mode,"policy_version":version,"assignment_revision":row.action.selection.expected_revision}),
        ),
        None,
        None,
        None,
        None,
    );
    Ok(row)
}

pub async fn get(
    db: &Database,
    actor: &str,
    agent: &str,
    node: &str,
    id: &str,
) -> AppResult<ContextReview> {
    team::maintained_agent(db, actor, agent).await?;
    let row = db
        .collection::<ContextReview>(model::ContextReview::COLLECTION_NAME)
        .find_one(doc! {"_id":id,"actor_id":actor,"action.agent_id":agent,"action.node_id":node})
        .await?
        .ok_or_else(|| AppError::NotFound("Machine context review not found".into()))?;
    if !super::org_service::resolve_owner_access(db, actor, &row.action.snapshot.owner_id)
        .await?
        .can_write()
    {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(row)
}

pub async fn commit(
    db: &Database,
    actor: &str,
    agent: &str,
    node: &str,
    id: &str,
) -> AppResult<ContextReview> {
    let row = get(db, actor, agent, node, id).await?;
    if row.status == "committed" {
        return Ok(row);
    }
    if !matches!(row.status.as_str(), "pending" | "approved") {
        return Err(stale());
    }
    db.collection::<Document>(model::ContextReview::COLLECTION_NAME)
        .update_one(
            doc! {"_id":id,"actor_id":actor,"status":"pending"},
            doc! {"$set":{"status":"approved","approved_at":bson::DateTime::now()}},
        )
        .await?;
    let applied = Box::pin(access::configure_reviewed(db, actor, &row.action, Some(id))).await;
    if let Err(error) = applied {
        let code = match &error {
            AppError::Conflict(_) => "machine_context_review_stale",
            AppError::MachineNotAllowed
            | AppError::MachinePermissionRevoked
            | AppError::Forbidden(_) => "authorization_changed",
            AppError::MachineAuthorityUnsupported => "machine_authority_upgrade_required",
            AppError::ValidationError(_) => "mode_unavailable",
            _ => "authorization_commit_failed",
        };
        db.collection::<Document>(model::ContextReview::COLLECTION_NAME)
            .update_one(
                doc! {"_id":id,"actor_id":actor,"status":"approved"},
                doc! {"$set":{"status":"failed","error_code":code}},
            )
            .await?;
        let current = get(db, actor, agent, node, id).await?;
        if current.status == "committed" {
            return Ok(current);
        }
        return Err(error);
    }
    super::audit_service::log_async(
        db.clone(),
        Some(actor.into()),
        "machine_context_mode_changed".into(),
        Some(
            serde_json::json!({"review_id":id,"agent_id":agent,"node_id":node,"mode":row.action.selection.mode,"source":"human_page_review"}),
        ),
        None,
        None,
        None,
        None,
    );
    get(db, actor, agent, node, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant_authority_tests::{Fixture, orchestrator_fixture};
    use nyxid_machine::authority::Capabilities;
    async fn machine(f: &Fixture) -> Node {
        access::ensure_indexes(&f.state.db).await.unwrap();
        let mut node = super::super::machine_integration_tests::ungranted_node_named(
            f,
            &f.owner,
            "mode-review-machine",
        )
        .await;
        let profile = node.machine.as_mut().unwrap();
        profile.authority_versions = vec![2];
        profile.authority_timing_v1 = true;
        profile.separation_choice = Some(nyxid_machine::context::Choice::SharedLegacy);
        f.state
            .db
            .collection::<Document>(node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&node.id},
                doc! {"$set":{"machine":bson::to_bson(&node.machine).unwrap()}},
            )
            .await
            .unwrap();
        node
    }
    async fn seed_policy(f: &Fixture, node: &Node, caps: Capabilities) {
        let policy = model::Policy {
            version: 2,
            revision: 7,
            assignments: std::collections::BTreeMap::from([(
                node.id.clone(),
                model::Assignment {
                    capabilities: caps,
                    mode: "separated".into(),
                    revision: 7,
                    legacy: false,
                    saved_login_ids: Some(vec![]),
                },
            )]),
        };
        f.state.db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME).update_one(doc!{"_id":&f.chat.agent_id},doc!{"$set":{"machine_access":bson::to_bson(&policy).unwrap(),"machine_node_ids":[&node.id]}}).await.unwrap();
    }
    async fn flag(f: &Fixture, key: &str, on: bool) {
        super::super::security_readiness_service::ready_test_replica(&f.state.db).await;
        super::super::feature_flag_service::set_platform_override(
            &f.state.db,
            key,
            &super::super::feature_flag_service::FlagTarget::User(f.owner.clone()),
            on,
            &f.owner,
        )
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn owner_mode_review_preserves_partial_and_all_off_with_editor_off_and_pending_ack() {
        for caps in [
            Capabilities::default(),
            Capabilities {
                files: true,
                ..Default::default()
            },
        ] {
            let f = orchestrator_fixture("mode_review_exact_grants").await;
            let node = machine(&f).await;
            seed_policy(&f, &node, caps).await;
            let logins = if caps.files {
                vec![uuid::Uuid::new_v4().to_string()]
            } else {
                vec![]
            };
            let login_field = format!("machine_access.assignments.{}.saved_login_ids", node.id);
            f.state
                .db
                .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&f.chat.agent_id},
                    doc! {"$set":{login_field:&logins,"saved_login_ids":&logins}},
                )
                .await
                .unwrap();
            let row = prepare(
                &f.state.db,
                &f.owner,
                &f.chat.agent_id,
                &node.id,
                Mode::SharedLegacy,
                model::CONTEXT_POLICY_VERSION,
                true,
            )
            .await
            .unwrap();
            assert_eq!(row.action.selection.capabilities, caps);
            assert_eq!(row.action.selection.saved_login_ids, Some(logins.clone()));
            let saved = commit(&f.state.db, &f.owner, &f.chat.agent_id, &node.id, &row.id)
                .await
                .unwrap();
            assert_eq!(saved.status, "committed");
            assert_eq!(saved.committed_revision, Some(8));
            let policy = access::policy(&f.state.db, &f.chat.agent_id).await.unwrap();
            assert_eq!(policy.assignments[&node.id].capabilities, caps);
            assert_eq!(policy.assignments[&node.id].saved_login_ids, Some(logins));
            assert_eq!(policy.assignments[&node.id].mode, "shared_legacy");
            assert_eq!(
                access::revocation_status(&f.state.db, &f.chat.agent_id, &[node.id.clone()])
                    .await
                    .unwrap()
                    .len(),
                1
            );
            let replay = commit(&f.state.db, &f.owner, &f.chat.agent_id, &node.id, &row.id)
                .await
                .unwrap();
            assert_eq!(replay.committed_revision, Some(8));
            assert_eq!(
                access::policy(&f.state.db, &f.chat.agent_id)
                    .await
                    .unwrap()
                    .revision,
                8
            );
            f.state.db.drop().await.unwrap();
        }
    }
    #[tokio::test]
    async fn owner_can_opt_into_verified_separation_without_enabling_or_broadening_tools() {
        let f = orchestrator_fixture("mode_review_separated_opt_in").await;
        let mut node = machine(&f).await;
        let evidence = super::super::machine_access_tests::supported_evidence();
        let profile = node.machine.as_mut().unwrap();
        profile.separated = Some(evidence.support());
        profile.separation_evidence = Some(evidence);
        f.state
            .db
            .collection::<Document>(node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&node.id},
                doc! {"$set":{"machine":bson::to_bson(&node.machine).unwrap()}},
            )
            .await
            .unwrap();
        let caps = Capabilities {
            files: true,
            ..Default::default()
        };
        seed_policy(&f, &node, caps).await;
        let assignment = format!("machine_access.assignments.{}", node.id);
        let mode_field = format!("{assignment}.mode");
        let login_field = format!("{assignment}.saved_login_ids");
        f.state
            .db
            .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.agent_id},
                doc! {"$set":{mode_field:"shared_legacy",login_field:bson::Bson::Null}},
            )
            .await
            .unwrap();
        flag(&f, access::CONTEXT_FLAG, true).await;
        let row = prepare(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &node.id,
            Mode::Separated,
            model::CONTEXT_POLICY_VERSION,
            true,
        )
        .await
        .unwrap();
        assert!(!row.action.snapshot.capability_edit_enabled);
        assert_eq!(row.action.selection.capabilities, caps);
        assert_eq!(row.action.selection.saved_login_ids, Some(vec![]));
        commit(&f.state.db, &f.owner, &f.chat.agent_id, &node.id, &row.id)
            .await
            .unwrap();
        let current = access::policy(&f.state.db, &f.chat.agent_id).await.unwrap();
        assert_eq!(current.assignments[&node.id].mode, "separated");
        assert_eq!(current.assignments[&node.id].capabilities, caps);
        assert_eq!(current.assignments[&node.id].saved_login_ids, Some(vec![]));
        f.state.db.drop().await.unwrap();
    }
    async fn mounted_commit(f: &Fixture, node: &str, review: &str) -> axum::http::StatusCode {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let token = crate::crypto::jwt::generate_access_token(
            &f.state.jwt_keys,
            &f.state.config,
            &uuid::Uuid::parse_str(&f.owner).unwrap(),
            "read write",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        private.with_state(f.state.clone()).oneshot(Request::post(format!(
            "/api/v1/assistant/nyxagent/agents/{}/machines/{node}/context-reviews/{review}/commit",f.chat.agent_id))
            .header("authorization",format!("Bearer {token}")).header("content-type","application/json")
            .body(Body::from("{}")).unwrap()).await.unwrap().status()
    }
    #[tokio::test]
    async fn concurrent_owner_reviews_cannot_commit_the_same_authority_revision_twice() {
        let f = orchestrator_fixture("mode_review_concurrent_commit").await;
        let node = machine(&f).await;
        let caps = Capabilities {
            files: true,
            ..Default::default()
        };
        seed_policy(&f, &node, caps).await;
        let mut reviews = Vec::new();
        for _ in 0..2 {
            reviews.push(
                prepare(
                    &f.state.db,
                    &f.owner,
                    &f.chat.agent_id,
                    &node.id,
                    Mode::SharedLegacy,
                    model::CONTEXT_POLICY_VERSION,
                    true,
                )
                .await
                .unwrap(),
            );
        }
        let (first, second) = tokio::join!(
            mounted_commit(&f, &node.id, &reviews[0].id),
            mounted_commit(&f, &node.id, &reviews[1].id)
        );
        let mut statuses = [first.as_u16(), second.as_u16()];
        statuses.sort();
        assert_eq!(
            statuses,
            [200, 409],
            "only one mounted reviewed revision may commit"
        );
        let policy = access::policy(&f.state.db, &f.chat.agent_id).await.unwrap();
        assert_eq!(policy.revision, 8);
        assert_eq!(policy.assignments[&node.id].capabilities, caps);
        assert_eq!(policy.assignments[&node.id].saved_login_ids, Some(vec![]));
        assert_eq!(
            f.state
                .db
                .collection::<Document>(model::ContextReview::COLLECTION_NAME)
                .count_documents(doc! {"actor_id":&f.owner,"status":"committed"})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>(model::OUTBOX)
                .count_documents(
                    doc! {"agent_id":&f.chat.agent_id,"node_id":&node.id,"revision":8_i64}
                )
                .await
                .unwrap(),
            1
        );
        f.state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn mode_review_rejects_concurrent_policy_flag_profile_expiry_and_owner_loss() {
        for drift in ["policy", "flag", "profile", "expiry", "owner"] {
            let f = orchestrator_fixture("mode_review_drift").await;
            let node = machine(&f).await;
            seed_policy(&f, &node, Capabilities::default()).await;
            let row = prepare(
                &f.state.db,
                &f.owner,
                &f.chat.agent_id,
                &node.id,
                Mode::SharedLegacy,
                model::CONTEXT_POLICY_VERSION,
                true,
            )
            .await
            .unwrap();
            match drift {
                "policy" => {
                    f.state
                        .db
                        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
                        .update_one(
                            doc! {"_id":&f.chat.agent_id},
                            doc! {"$inc":{"machine_access.revision":1_i64}},
                        )
                        .await
                        .unwrap();
                }
                "flag" => {
                    flag(&f, access::FLAG, true).await;
                    flag(&f, access::FLAG, false).await;
                }
                "profile" => {
                    f.state
                        .db
                        .collection::<Document>(node::COLLECTION_NAME)
                        .update_one(
                            doc! {"_id":&node.id},
                            doc! {"$set":{"machine.separation_evidence":{"version":999}}},
                        )
                        .await
                        .unwrap();
                }
                "expiry" => {
                    f.state.db.collection::<Document>(model::ContextReview::COLLECTION_NAME).update_one(doc!{"_id":&row.id},doc!{"$set":{"action.snapshot.expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
                }
                _ => {
                    f.state
                        .db
                        .collection::<Document>(crate::models::user::COLLECTION_NAME)
                        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"is_active":false}})
                        .await
                        .unwrap();
                }
            }
            let status = mounted_commit(&f, &node.id, &row.id).await;
            assert!(
                !status.is_success(),
                "mounted {drift} unexpectedly committed"
            );
            if drift != "owner" {
                assert_eq!(status, axum::http::StatusCode::CONFLICT, "{drift}");
            }
            assert!(
                commit(&f.state.db, &f.owner, &f.chat.agent_id, &node.id, &row.id)
                    .await
                    .is_err(),
                "{drift}"
            );
            let current = f
                .state
                .db
                .collection::<ContextReview>(model::ContextReview::COLLECTION_NAME)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            assert_ne!(current.status, "committed");
            let policy = access::policy(&f.state.db, &f.chat.agent_id).await.unwrap();
            assert_eq!(policy.assignments[&node.id].mode, "separated");
            assert_eq!(
                policy.assignments[&node.id].capabilities,
                Capabilities::default()
            );
            f.state.db.drop().await.unwrap();
        }
    }
    #[tokio::test]
    async fn all_new_hosted_assignments_are_membership_only_for_every_choice_and_flag_pair() {
        let f = orchestrator_fixture("hosted_membership_only").await;
        let node = machine(&f).await;
        let mut managed = super::super::remote_machine_service::CreateMachineInput::fixed(
            &f.owner, true, "fixture",
        )
        .preview(&node.id, Utc::now());
        f.state
            .db
            .collection::<crate::models::remote_machine::ManagedMachine>(
                crate::models::remote_machine::COLLECTION_NAME,
            )
            .insert_one(&managed)
            .await
            .unwrap();
        use axum::{
            body::{Body, to_bytes},
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        let token = crate::crypto::jwt::generate_access_token(
            &f.state.jwt_keys,
            &f.state.config,
            &uuid::Uuid::parse_str(&f.owner).unwrap(),
            "read write",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let request = |method: &str, path: String, body: serde_json::Value| {
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let response=app.clone().oneshot(request("POST","/api/v1/assistant/nyxagent/agents".into(),
            serde_json::json!({"name":"matrix-specialist","description":"Mounted membership contract","machines":[]}))).await.unwrap();
        let status = response.status();
        let created: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let agent = created["id"].as_str().unwrap();
        for editing in [false, true] {
            for contexts in [false, true] {
                for choice in [
                    nyxid_machine::context::Choice::Automatic,
                    nyxid_machine::context::Choice::RequireSeparated,
                    nyxid_machine::context::Choice::SharedLegacy,
                ] {
                    flag(&f, access::FLAG, editing).await;
                    flag(&f, access::CONTEXT_FLAG, contexts).await;
                    managed.configuration.separation_choice = choice;
                    f.state.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME).update_one(doc!{"_id":&node.id},doc!{"$set":{"configuration.separation_choice":bson::to_bson(&choice).unwrap()}}).await.unwrap();
                    f.state.db.collection::<Document>(node::COLLECTION_NAME).update_one(doc!{"_id":&node.id},doc!{"$set":{"machine.separation_choice":bson::to_bson(&choice).unwrap()}}).await.unwrap();
                    let rows =
                        access::new_assignments(&f.state.db, &f.owner, &[node.id.clone()], 4)
                            .await
                            .unwrap();
                    assert_eq!(
                        rows[&node.id].capabilities,
                        Capabilities::default(),
                        "{editing}/{contexts}/{choice:?}"
                    );
                    assert_eq!(rows[&node.id].saved_login_ids, Some(vec![]));
                    assert!(!rows[&node.id].legacy);
                    // Exercise the actual membership producer and HTTP
                    // projection for every choice/flag pair, including re-add.
                    for machines in [vec![], vec![node.id.clone()]] {
                        let response=app.clone().oneshot(request("PUT",format!("/api/v1/assistant/nyxagent/agents/{agent}/grants"),
                            serde_json::json!({"machines":machines,"services":[],"account_read":false}))).await.unwrap();
                        assert_eq!(
                            response.status(),
                            StatusCode::OK,
                            "{editing}/{contexts}/{choice:?}"
                        );
                    }
                    let response = app
                        .clone()
                        .oneshot(request(
                            "GET",
                            format!("/api/v1/assistant/nyxagent/agents/{agent}/machines"),
                            serde_json::json!({}),
                        ))
                        .await
                        .unwrap();
                    let status = response.status();
                    let body: serde_json::Value = serde_json::from_slice(
                        &to_bytes(response.into_body(), 1024 * 1024).await.unwrap(),
                    )
                    .unwrap();
                    assert_eq!(status, StatusCode::OK, "{body}");
                    let option = body
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|v| v["node_id"] == node.id)
                        .unwrap();
                    assert_eq!(option["membership"], true);
                    assert_eq!(option["legacy"], false);
                    assert_eq!(option["saved_login_ids"], serde_json::json!([]));
                    assert_eq!(
                        option["capabilities"],
                        serde_json::to_value(Capabilities::default()).unwrap()
                    );
                    assert_eq!(option["capability_edit_enabled"], editing);
                    assert_eq!(option["context_setup_enabled"], contexts);
                    if choice != nyxid_machine::context::Choice::SharedLegacy {
                        assert_eq!(rows[&node.id].mode, model::SELECTION_REQUIRED);
                    }
                }
            }
        }
        f.state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn hosted_mode_review_reuses_current_config_and_rejects_changed_target() {
        use crate::models::remote_machine::{
            MachineConfigurationEvidence, ManagedMachine, ResourceState,
        };
        let f = orchestrator_fixture("hosted_review_config_binding").await;
        let mut node = machine(&f).await;
        seed_policy(&f, &node, Capabilities::default()).await;
        let mut managed = super::super::remote_machine_service::CreateMachineInput::fixed(
            &f.owner, true, "fixture",
        )
        .preview(&node.id, Utc::now());
        managed.resource_state = ResourceState::Ready;
        managed.writer_admitted = true;
        managed.runtime_generation = 1;
        managed.credential_epoch = 1;
        let profile = node.machine.as_mut().unwrap();
        profile.version = nyxid_machine::PROTOCOL_VERSION;
        profile.shell = true;
        profile.files = true;
        profile.browser = Some(false);
        profile.computer = false;
        profile.roots = vec!["/workspace".into()];
        profile.commands_isolated = Some(true);
        profile.managed_configuration = Some(nyxid_machine::managed::ConfigurationReport {
            configuration: managed.configuration.clone(),
            digest: managed.configuration.digest().unwrap(),
            runtime_generation: 1,
        });
        let (sender, _receiver) = tokio::sync::mpsc::channel(16);
        let connection =
            crate::test_utils::register_test_node_connection(&f.state, &node.id, sender).await;
        f.state.db.collection::<Document>(node::COLLECTION_NAME).update_one(doc!{"_id":&node.id},doc!{"$set":{"status":"online","connection_owner.capabilities_resolved":true,"machine":bson::to_bson(&*profile).unwrap()}}).await.unwrap();
        managed.configuration_evidence = Some(MachineConfigurationEvidence {
            configuration_revision: 1,
            configuration_digest: managed.configuration.digest().unwrap(),
            runtime_generation: 1,
            credential_epoch: 1,
            runtime_id: profile.runtime_id.clone(),
            connection_id: connection,
            observed_at: Utc::now(),
        });
        let collection = f
            .state
            .db
            .collection::<ManagedMachine>(crate::models::remote_machine::COLLECTION_NAME);
        collection.insert_one(&managed).await.unwrap();
        // A page review can also create membership. A missing hosted assignment
        // must start with explicit no-login scope, never inherited access.
        f.state
            .db
            .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.agent_id},
                doc! {"$set":{"machine_access.assignments":Document::new(),"machine_node_ids":[]}},
            )
            .await
            .unwrap();
        // Use the mounted first-party API for the complete successful
        // prepare/read/commit journey and the dedicated target response DTO.
        use axum::{
            body::{Body, to_bytes},
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        let token = crate::crypto::jwt::generate_access_token(
            &f.state.jwt_keys,
            &f.state.config,
            &uuid::Uuid::parse_str(&f.owner).unwrap(),
            "read write",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let base = format!(
            "/api/v1/assistant/nyxagent/agents/{}/machines/{}",
            f.chat.agent_id, node.id
        );
        let request = |method: &str, path: String, body: serde_json::Value| {
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let response=app.clone().oneshot(request("POST",format!("{base}/context-prepare"),
            serde_json::json!({"mode":"shared_legacy","policy_version":model::CONTEXT_POLICY_VERSION,"acknowledge_separation":true}))).await.unwrap();
        let status = response.status();
        let prepared: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(status, StatusCode::OK, "{prepared}");
        assert_eq!(
            prepared["capabilities"],
            serde_json::to_value(Capabilities::default()).unwrap()
        );
        assert_eq!(prepared["saved_login_ids"], serde_json::json!([]));
        assert_eq!(
            prepared["machine_configuration_target"],
            serde_json::json!({"revision":1,"digest":managed.configuration.digest().unwrap(),"runtime_generation":1,"credential_epoch":1})
        );
        let review_id = prepared["review_id"].as_str().unwrap();
        let response = app
            .clone()
            .oneshot(request(
                "GET",
                format!("{base}/context-reviews/{review_id}"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let read: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(
            read["machine_configuration_target"],
            prepared["machine_configuration_target"]
        );
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(request(
                    "POST",
                    format!("{base}/context-reviews/{review_id}/commit"),
                    serde_json::json!({}),
                ))
                .await
                .unwrap();
            let status = response.status();
            let saved: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                    .unwrap();
            assert_eq!(status, StatusCode::OK, "{saved}");
            assert_eq!(saved["status"], "committed");
            assert_eq!(saved["committed_revision"], 8);
            assert_eq!(
                saved["environment_applied"], false,
                "commit alone cannot confirm both chains"
            );
        }
        let policy = access::policy(&f.state.db, &f.chat.agent_id).await.unwrap();
        assert_eq!(
            policy.assignments[&node.id].capabilities,
            Capabilities::default()
        );
        assert_eq!(policy.assignments[&node.id].saved_login_ids, Some(vec![]));
        let after = collection
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.configuration, managed.configuration,
            "Agent-only change never writes a machine target or restarts it"
        );
        assert_eq!(after.runtime_generation, 1);
        let review = prepare(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &node.id,
            Mode::SharedLegacy,
            model::CONTEXT_POLICY_VERSION,
            true,
        )
        .await
        .unwrap();
        collection
            .update_one(
                doc! {"_id":&node.id},
                doc! {"$inc":{"configuration.revision":1_i64}},
            )
            .await
            .unwrap();
        assert_eq!(
            mounted_commit(&f, &node.id, &review.id).await,
            axum::http::StatusCode::CONFLICT
        );
        assert!(
            commit(
                &f.state.db,
                &f.owner,
                &f.chat.agent_id,
                &node.id,
                &review.id
            )
            .await
            .is_err()
        );
        assert_eq!(
            access::policy(&f.state.db, &f.chat.agent_id)
                .await
                .unwrap()
                .revision,
            8
        );
        f.state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn owner_card_reprepare_after_dependency_drift_uses_a_new_exact_snapshot() {
        let f = orchestrator_fixture("mode_review_card_refresh").await;
        let node = machine(&f).await;
        seed_policy(&f, &node, Capabilities::default()).await;
        let request = || {
            crate::handlers::machine_access::request_context(
                axum::extract::State(f.state.clone()),
                crate::test_utils::test_auth_user(&f.owner),
                axum::extract::Path((f.chat.agent_id.clone(), node.id.clone())),
                axum::Json(crate::handlers::machine_access::ContextRequestBody {
                    mode: Mode::SharedLegacy,
                    policy_version: model::CONTEXT_POLICY_VERSION.into(),
                    acknowledge_separation: true,
                }),
            )
        };
        let first = serde_json::to_value(request().await.unwrap().0).unwrap();
        flag(&f, access::FLAG, true).await;
        flag(&f, access::FLAG, false).await;
        let second = serde_json::to_value(request().await.unwrap().0).unwrap();
        assert_ne!(
            first["acknowledgement_id"], second["acknowledgement_id"],
            "new review cannot reuse a pending card's stale snapshot"
        );
        for (response, valid) in [(first, false), (second, true)] {
            let row = f
                .state
                .db
                .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                    crate::models::assistant_acknowledgement::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":response["acknowledgement_id"].as_str().unwrap()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                validate_action(&f.state.db, &f.owner, row.machine_context.as_ref().unwrap())
                    .await
                    .is_ok(),
                valid
            );
        }
        f.state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn mounted_owner_review_rejects_client_selection_delegation_wrong_policy_and_stale_commit()
     {
        use axum::{
            body::{Body, to_bytes},
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        let f = orchestrator_fixture("mounted_context_review").await;
        let node = machine(&f).await;
        seed_policy(
            &f,
            &node,
            Capabilities {
                files: true,
                ..Default::default()
            },
        )
        .await;
        let token = crate::crypto::jwt::generate_access_token(
            &f.state.jwt_keys,
            &f.state.config,
            &uuid::Uuid::parse_str(&f.owner).unwrap(),
            "read write",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let path = format!(
            "/api/v1/assistant/nyxagent/agents/{}/machines/{}",
            f.chat.agent_id, node.id
        );
        let post = |path: String, body: serde_json::Value| {
            Request::post(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        for body in [
            serde_json::json!({"mode":"shared_legacy","policy_version":model::CONTEXT_POLICY_VERSION,"acknowledge_separation":true,"selection":{"capabilities":{"shell":true}}}),
            serde_json::json!({"mode":"shared_legacy","policy_version":model::CONTEXT_POLICY_VERSION}),
            serde_json::json!({"mode":"automatic","policy_version":model::CONTEXT_POLICY_VERSION,"acknowledge_separation":true}),
        ] {
            let response = app
                .clone()
                .oneshot(post(format!("{path}/context-prepare"), body))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        }
        let response=app.clone().oneshot(post(format!("{path}/context-prepare"),serde_json::json!({"mode":"shared_legacy","policy_version":"forged","acknowledge_separation":true}))).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response=app.clone().oneshot(post(format!("{path}/context-prepare"),serde_json::json!({"mode":"shared_legacy","policy_version":model::CONTEXT_POLICY_VERSION,"acknowledge_separation":true}))).await.unwrap();
        let status = response.status();
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["capabilities"]["files"], true);
        assert_eq!(body["capabilities"]["shell"], false);
        assert_eq!(body["environment_applied"], false);
        let review = body["review_id"].as_str().unwrap();
        let forged_commit = app
            .clone()
            .oneshot(post(
                format!("{path}/context-reviews/{review}/commit"),
                serde_json::json!({"selection":{"capabilities":{"shell":true}}}),
            ))
            .await
            .unwrap();
        assert_eq!(forged_commit.status(), StatusCode::UNPROCESSABLE_ENTITY);
        flag(&f, access::FLAG, true).await;
        let delegated_token = crate::crypto::jwt::generate_delegated_access_token(
            &f.state.jwt_keys,
            &f.state.config,
            &uuid::Uuid::parse_str(&f.owner).unwrap(),
            "account:read",
            "aevatar",
            60,
            None,
        )
        .unwrap();
        for suffix in [
            "context-prepare".to_owned(),
            format!("context-reviews/{review}/commit"),
        ] {
            let response=app.clone().oneshot(Request::post(format!("{path}/{suffix}"))
                .header("authorization",format!("Bearer {delegated_token}"))
                .header("content-type","application/json")
                .body(Body::from(serde_json::json!({"mode":"shared_legacy","policy_version":model::CONTEXT_POLICY_VERSION,"acknowledge_separation":true}).to_string())).unwrap()).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "delegated {suffix}"
            );
        }
        let response = app
            .oneshot(post(
                format!("{path}/context-reviews/{review}/commit"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        // The auth extractor's verified first-party boundary is tested at the handler too.
        let mut delegated = f.auth.clone();
        delegated.oauth_client_id = Some(uuid::Uuid::new_v4().to_string());
        let denied = crate::handlers::machine_access::prepare_context(
            axum::extract::State(f.state.clone()),
            delegated,
            axum::extract::Path((f.chat.agent_id.clone(), node.id)),
            axum::Json(crate::handlers::machine_access::ContextRequestBody {
                mode: Mode::SharedLegacy,
                policy_version: model::CONTEXT_POLICY_VERSION.into(),
                acknowledge_separation: true,
            }),
        )
        .await;
        assert!(matches!(
            denied,
            Err(AppError::ForbiddenWithGuidance { .. })
        ));
        f.state.db.drop().await.unwrap();
    }
}
