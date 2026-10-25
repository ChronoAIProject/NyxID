//! Hosted creation choices and one-time specialist assignment. Local capabilities
//! are an execution ceiling, never a replacement for existing agent authority.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent,
        remote_machine::{
            COLLECTION_NAME as MACHINES, HostedMachineChoices, ManagedMachine,
            SafeManagedMachineView,
        },
    },
};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use serde_json::Value;

#[cfg(test)]
mod contract_tests {
    use super::*;
    use nyxid_machine::{
        MachineProfile,
        managed::{CONFIG_VERSION, ConfigurationReport},
    };

    #[test]
    fn configuration_requires_exact_report_choice_ceiling_revision_and_generation() {
        let mut machine = super::super::remote_machine_service::CreateMachineInput::fixed(
            "owner", true, "fixture",
        )
        .preview("node", chrono::Utc::now());
        machine.runtime_generation = 1;
        let mut profile = MachineProfile {
            version: nyxid_machine::PROTOCOL_VERSION,
            runtime_id: uuid::Uuid::new_v4().to_string(),
            shell: true,
            files: true,
            roots: vec!["/workspace".into()],
            authority_versions: vec![2],
            commands_isolated: Some(true),
            separation_choice: Some(machine.configuration.separation_choice),
            managed_configuration: Some(ConfigurationReport {
                configuration: machine.configuration.clone(),
                digest: machine.configuration.digest().unwrap(),
                runtime_generation: machine.runtime_generation as u64,
            }),
            ..Default::default()
        };
        assert!(report_matches(&machine, &profile));
        for invalid in 0..6 {
            let mut stale = profile.clone();
            let report = stale.managed_configuration.as_mut().unwrap();
            match invalid {
                0 => report.runtime_generation += 1,
                1 => report.configuration.revision += 1,
                2 => report.configuration.version = CONFIG_VERSION + 1,
                3 => report.digest = "forged".into(),
                4 => stale.separation_choice = Some(nyxid_machine::context::Choice::Automatic),
                _ => stale.files = false,
            }
            assert!(!report_matches(&machine, &stale), "case {invalid}");
        }
        profile.managed_configuration = None;
        assert!(!report_matches(&machine, &profile));
        assert_eq!(configuration_view(&machine, None).state, "pending");
    }

    #[test]
    fn hosted_membership_defaults_never_inherit_ceiling_or_login_scope() {
        for choice in [
            nyxid_machine::context::Choice::Automatic,
            nyxid_machine::context::Choice::RequireSeparated,
            nyxid_machine::context::Choice::SharedLegacy,
        ] {
            for contexts in [false, true] {
                let profile = MachineProfile {
                    shell: true,
                    files: true,
                    browser: Some(true),
                    computer: true,
                    // The runtime may not replace a recorded separated choice
                    // with its own shared-mode assertion.
                    separation_choice: Some(nyxid_machine::context::Choice::SharedLegacy),
                    ..Default::default()
                };
                let assignment = hosted_assignment(&profile, choice, 9, contexts);
                assert_eq!(assignment.capabilities, Default::default());
                assert_eq!(assignment.saved_login_ids, Some(vec![]));
                assert!(!assignment.legacy);
                assert_eq!(
                    assignment.mode,
                    if choice == nyxid_machine::context::Choice::SharedLegacy {
                        "shared_legacy"
                    } else {
                        crate::models::machine_access::SELECTION_REQUIRED
                    }
                );
            }
        }
    }
}

pub fn hosted_assignment(
    profile: &nyxid_machine::MachineProfile,
    choice: nyxid_machine::context::Choice,
    revision: i64,
    contexts: bool,
) -> crate::models::machine_access::Assignment {
    // Only persisted owner consent selects a hosted default. A Node profile
    // supplies support evidence, never replacement shared-mode consent.
    let mut selected = profile.clone();
    selected.separation_choice = Some(choice);
    let mut assignment =
        super::machine_access_service::selected_assignment(&selected, revision, true, contexts);
    assignment.capabilities = Default::default();
    assignment.saved_login_ids = Some(vec![]);
    assignment.legacy = false;
    assignment
}

pub async fn hosted_node_choices(
    db: &Database,
    ids: &[String],
) -> AppResult<std::collections::HashMap<String, nyxid_machine::context::Choice>> {
    use futures::TryStreamExt;
    let rows: Vec<ManagedMachine> = db
        .collection::<ManagedMachine>(MACHINES)
        .find(doc! {"_id":{"$in":ids}})
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.node_id, row.configuration.separation_choice))
        .collect())
}

pub fn report_matches(machine: &ManagedMachine, profile: &nyxid_machine::MachineProfile) -> bool {
    let Some(report) = &profile.managed_configuration else {
        return false;
    };
    let selected = |cap: &str| machine.capabilities.iter().any(|c| c == cap);
    report.runtime_generation > 0
        && report.configuration == machine.configuration
        && report.configuration.capabilities == machine.capabilities
        && report
            .configuration
            .digest()
            .is_ok_and(|digest| digest == report.digest)
        && u64::try_from(machine.runtime_generation).ok() == Some(report.runtime_generation)
        && profile.separation_choice == Some(machine.configuration.separation_choice)
        && profile.shell == selected("shell")
        && profile.files == selected("files")
        && profile.browser_enabled() == selected("browser")
        && profile.computer == selected("computer")
}

pub fn configuration_view(
    machine: &ManagedMachine,
    node: Option<&crate::models::node::Node>,
) -> crate::models::remote_machine::SafeMachineConfigurationView {
    let digest = machine.configuration.digest().unwrap_or_default();
    let applied = node.is_some_and(|node| {
        let Some(profile) = node.machine.as_ref() else {
            return false;
        };
        let Some(connection) = node
            .connection_owner
            .as_ref()
            .filter(|c| c.capabilities_resolved && c.is_live_at(chrono::Utc::now()))
        else {
            return false;
        };
        node.is_active
            && node.user_id == machine.owner_id
            && node.status == crate::models::node::NodeStatus::Online
            && machine.writer_admitted
            && machine.resource_state == crate::models::remote_machine::ResourceState::Ready
            && super::remote_machine_manager::accepts_selected_profile(machine, profile)
            && machine.configuration_evidence.as_ref().is_some_and(|e| {
                e.configuration_revision == machine.configuration.revision
                    && e.configuration_digest == digest
                    && e.runtime_generation == machine.runtime_generation
                    && e.credential_epoch == machine.credential_epoch
                    && e.runtime_id == profile.runtime_id
                    && e.connection_id == connection.connection_id
            })
    });
    crate::models::remote_machine::SafeMachineConfigurationView {
        version: machine.configuration.version,
        revision: machine.configuration.revision,
        digest,
        runtime_generation: machine.runtime_generation,
        state: if applied { "applied" } else { "pending" }.into(),
        reason: (!applied).then(|| {
            if machine.bootstrap_delivery_id.is_none() {
                "bootstrap_pending"
            } else {
                "current_runtime_configuration_evidence_required"
            }
            .into()
        }),
    }
}

pub fn normalize_request(body: &mut Value) -> AppResult<HostedMachineChoices> {
    let mut choices: HostedMachineChoices = serde_json::from_value(body.clone())
        .map_err(|_| AppError::ValidationError("Invalid hosted Machine choices".into()))?;
    if choices.separation_policy_version != crate::models::remote_machine::SEPARATION_POLICY_VERSION
        || !choices.acknowledge_separation
    {
        return Err(AppError::ValidationError(
            "Current separation policy and explicit acknowledgement are required".into(),
        ));
    }
    super::machine_setup_service::validate_machine_name(&choices.name)?;
    choices.capabilities.sort();
    choices.capabilities.dedup();
    if choices.capabilities.is_empty()
        || choices
            .capabilities
            .iter()
            .any(|cap| !matches!(cap.as_str(), "shell" | "files" | "browser" | "computer"))
        || (choices.capabilities.iter().any(|cap| cap == "computer")
            && !choices.capabilities.iter().any(|cap| cap == "browser"))
    {
        return Err(AppError::ValidationError(
            "Choose at least one Machine capability; Computer requires Browser".into(),
        ));
    }
    if let Some(id) = choices.specialist_id.as_mut() {
        *id = uuid::Uuid::parse_str(id)
            .map_err(|_| AppError::ValidationError("Choose a valid specialist".into()))?
            .to_string();
    }
    let object = body
        .as_object_mut()
        .ok_or_else(|| AppError::ValidationError("Invalid hosted Machine request".into()))?;
    object.insert("name".into(), serde_json::json!(choices.name));
    object.insert(
        "capabilities".into(),
        serde_json::json!(choices.capabilities),
    );
    object.insert(
        "specialist_id".into(),
        serde_json::json!(choices.specialist_id),
    );
    Ok(choices)
}

pub async fn validate_specialist(db: &Database, owner: &str, id: Option<&str>) -> AppResult<()> {
    if let Some(id) = id {
        let agent = super::assistant_team_service::live_specialist(db, owner, id)
            .await
            .map_err(|error| match error {
                AppError::Conflict(_) => {
                    AppError::ValidationError("Choose a live specialist".into())
                }
                other => other,
            })?;
        // Existing organization agent grants cannot receive a personal Machine.
        if agent.user_id != owner {
            return Err(AppError::Forbidden(
                "Choose a personal specialist for this personal Machine".into(),
            ));
        }
        super::org_agent_service::require_maintain(db, owner, &agent).await?;
    }
    Ok(())
}

/// Retry only the initial assignment. Once delivered, later user revocations are
/// never repaired back into grants by a create replay, heartbeat or resume.
pub async fn assign_specialist(db: &Database, node_id: &str) -> AppResult<()> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    let transaction_db = db.clone();
    let transaction_node = node_id.clone();
    let result = db.client().start_session().await?.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let Some(machine) = transaction_db.collection::<ManagedMachine>(MACHINES)
                    .find_one(doc!{"_id":&transaction_node,"specialist_assignment_state":"pending","resource_state":"ready","writer_admitted":true})
                    .session(&mut *session).await? else {return Ok(())};
                let Some(specialist) = machine.specialist_id.as_deref() else {return Ok(())};
                super::remote_machine_service::ensure_owner_eligible_in_session(&transaction_db, session, &machine.owner_id).await?;
                validate_specialist(&transaction_db, &machine.owner_id, Some(specialist)).await?;
                let agent = Box::pin(super::assistant_team_service::apply_grants_in_session(
                    &transaction_db, &machine.owner_id, specialist,
                    &super::assistant_team_service::GrantChange::Machine {
                        base: Box::new(super::assistant_team_service::GrantChange::Add(Default::default())),
                        machines: Some(vec![machine.node_id.clone()]), logins: None,
                        mode: super::assistant_team_service::MachineGrantMode::Add,
                    }, session,
                )).await?;
                if !agent.machine_node_ids.contains(&machine.node_id) {
                    return Err(AppError::Internal("Machine assignment was not persisted".into()));
                }
                let updated = transaction_db.collection::<Document>(MACHINES).update_one(
                    doc!{"_id":&machine.node_id,"revision":machine.revision,"specialist_assignment_state":"pending","resource_state":"ready","writer_admitted":true},
                    doc!{"$set":{"specialist_assignment_state":"assigned","specialist_assignment_error":bson::Bson::Null,"updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64}},
                ).session(&mut *session).await?;
                if updated.matched_count != 1 {return Err(AppError::Conflict("Machine changed during specialist assignment".into()));}
                super::remote_machine_service::append_owner_audit(&transaction_db, session, &machine.owner_id,
                    "machine_specialist_assigned", &machine.node_id, machine.active_operation_id.as_deref().unwrap_or_default()).await?;
                Ok(())
            }.await;
            super::api_key_mutation_service::transaction_result(result)
        }).await.map_err(super::api_key_mutation_service::map_transaction_error);
    match result {
        Err(AppError::NotFound(_))
        | Err(AppError::Forbidden(_))
        | Err(AppError::ValidationError(_)) => {
            db.collection::<Document>(MACHINES).update_one(
                doc!{"_id":&node_id,"specialist_assignment_state":"pending"},
                doc!{"$set":{"specialist_assignment_state":"failed","specialist_assignment_error":"specialist_assignment_unavailable","updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64}},
            ).await?;
            Ok(())
        }
        other => other,
    }
}

/// Read current permissions rather than treating an old assignment receipt as
/// live authorization. This is used only by hosted management projections.
pub async fn safe_view(
    db: &Database,
    machine: &ManagedMachine,
) -> AppResult<SafeManagedMachineView> {
    let mut view = super::remote_machine_service::safe_view(machine);
    let node = db
        .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id":&machine.node_id})
        .await?;
    view.machine_configuration = configuration_view(machine, node.as_ref());
    if machine.specialist_assignment_state != "assigned" {
        return Ok(view);
    }
    let Some(id) = machine.specialist_id.as_deref() else {
        return Ok(view);
    };
    let agent = db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc!{"_id":id,"user_id":&machine.owner_id,"kind":"specialist","destroyed_at":bson::Bson::Null}).await?;
    let assignment = agent
        .as_ref()
        .filter(|a| a.machine_node_ids.contains(&machine.node_id))
        .and_then(|a| a.machine_access.as_ref())
        .and_then(|p| p.assignments.get(&machine.node_id));
    if assignment.is_none() {
        view.specialist_assignment_state = "not_assigned".into();
    }
    Ok(view)
}
