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

pub fn normalize_request(body: &mut Value) -> AppResult<HostedMachineChoices> {
    let mut choices: HostedMachineChoices = serde_json::from_value(body.clone())
        .map_err(|_| AppError::ValidationError("Invalid hosted Machine choices".into()))?;
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
    let complete = assignment.is_some_and(|a| {
        machine.capabilities.iter().all(|cap| match cap.as_str() {
            "shell" => a.capabilities.shell,
            "files" => a.capabilities.files,
            "browser" => a.capabilities.browser,
            "computer" => a.capabilities.computer,
            _ => false,
        })
    });
    if !complete {
        view.specialist_assignment_state = "permissions_required".into();
    }
    Ok(view)
}
