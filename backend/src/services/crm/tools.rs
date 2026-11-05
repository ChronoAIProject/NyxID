//! Native CRM tools. Payloads describe data; authority and provenance come from
//! the live conversation key and admitted turn, including scheduled turns.
use super::{activities, objects, query, records, transfer, views};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS},
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        crm_record::{CrmActor, CrmActorKind, CrmSource},
    },
    services::{assistant_acknowledgement_service as acks, mcp_service::McpToolEndpoint},
};
use mongodb::bson::doc;
use serde_json::{Value, json};

#[derive(Clone)]
struct MutationAdmission {
    user_id: String,
    agent_id: String,
    agent_owner_id: String,
    session_id: Option<String>,
    conversation_id: String,
    turn_id: String,
    api_key_id: String,
    grants: mongodb::bson::Bson,
    owner: String,
    schema: bool,
}
tokio::task_local! { static MUTATION_ADMISSION: MutationAdmission; }

/// Serialize native effects with key/turn/grant/member revocation in the same transaction.
pub(crate) async fn fence_admission(
    db: &mongodb::Database,
    owner: &str,
    tx: &mut mongodb::ClientSession,
) -> AppResult<()> {
    use mongodb::bson::{Bson, Document};
    let Ok(admission) = MUTATION_ADMISSION.try_with(Clone::clone) else {
        return Ok(());
    };
    if admission.owner != owner {
        return Err(AppError::Forbidden("CRM owner authority changed".into()));
    }
    let guards = [
        (
            AGENTS,
            if admission.grants.as_array().is_some_and(Vec::is_empty) {
                doc! {"_id":&admission.agent_id,"user_id":&admission.agent_owner_id,"destroyed_at":Bson::Null,"$or":[{"grants.crm":Bson::Null},{"grants.crm":&admission.grants}]}
            } else {
                doc! {"_id":&admission.agent_id,"user_id":&admission.agent_owner_id,"destroyed_at":Bson::Null,"grants.crm":&admission.grants}
            },
        ),
        (
            CONVERSATIONS,
            doc! {"_id":&admission.conversation_id,"user_id":&admission.user_id,"credential_api_key_id":&admission.api_key_id,"nyxagent_session_id":&admission.session_id,"active_turn.turn_id":&admission.turn_id,"active_turn.stop_requested":{"$ne":true},"$expr":{"$gt":[{"$ifNull":["$active_turn.lease_expires_at",{"$add":["$active_turn.started_at",crate::services::assistant_nyxagent::ACTIVE_TURN_TTL_SECS * 1000]}]},"$$NOW"]}},
        ),
        (
            crate::models::api_key::COLLECTION_NAME,
            doc! {"_id":&admission.api_key_id,"user_id":&admission.user_id,"is_active":true,"$or":[{"expires_at":Bson::Null},{"expires_at":{"$gt":mongodb::bson::DateTime::now()}}]},
        ),
        (
            crate::models::assistant_agent_credential::COLLECTION_NAME,
            doc! {"conversation_id":&admission.conversation_id,"user_id":&admission.user_id,"api_key_id":&admission.api_key_id},
        ),
        (
            crate::models::user::COLLECTION_NAME,
            doc! {"_id":&admission.user_id,"is_active":true,"user_type":"person"},
        ),
    ];
    for (collection, filter) in guards {
        if db
            .collection::<Document>(collection)
            .update_one(filter, doc! {"$inc":{"crm_write_fence":1_i64}})
            .session(&mut *tx)
            .await?
            .matched_count
            != 1
        {
            return Err(AppError::Forbidden(
                "CRM execution authority changed".into(),
            ));
        }
    }
    if owner != admission.user_id {
        let roles = if admission.schema {
            vec!["owner", "admin"]
        } else {
            vec!["owner", "admin", "member"]
        };
        if db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(doc! {"org_user_id":owner,"member_user_id":&admission.user_id,"revoked_at":Bson::Null,"role":{"$in":roles}},doc! {"$inc":{"crm_write_fence":1_i64}})
            .session(&mut *tx).await?.matched_count != 1 { return Err(AppError::Forbidden("CRM owner membership changed".into())); }
    }
    Ok(())
}

pub const NAMES: &[&str] = &[
    "crm_present",
    "crm_create_object",
    "crm_schema",
    "crm_query",
    "crm_aggregate",
    "crm_record",
    "crm_upsert_record",
    "crm_patch_record",
    "crm_archive_record",
    "crm_restore_record",
    "crm_add_note",
    "crm_activities",
    "crm_views",
    "crm_view",
    "crm_save_view",
    "crm_update_view",
    "crm_delete_view",
    "crm_add_field",
    "crm_update_field",
];

pub fn action_class(name: &str) -> Option<acks::OwnerActionClass> {
    use acks::OwnerActionClass::*;
    Some(match name {
        "crm_schema" | "crm_query" | "crm_aggregate" | "crm_record" | "crm_activities"
        | "crm_views" | "crm_view" | "crm_present" => Read,
        "crm_upsert_record" | "crm_patch_record" | "crm_restore_record" | "crm_add_note"
        | "crm_save_view" | "crm_update_view" => Change,
        "crm_archive_record" | "crm_delete_view" => Destructive,
        "crm_add_field" | "crm_update_field" | "crm_create_object" => Sensitive,
        _ => return None,
    })
}

pub fn annotation_hints(name: &str) -> Option<(bool, bool, bool)> {
    use acks::OwnerActionClass;
    Some(match action_class(name)? {
        OwnerActionClass::Read => (true, false, false),
        OwnerActionClass::Destructive => (false, true, false),
        OwnerActionClass::Change | OwnerActionClass::Sensitive => (false, false, false),
    })
}

pub async fn available(db: &mongodb::Database, chat: &acks::ChatAuthority) -> AppResult<bool> {
    if chat.guest
        || chat.collaboration_thread
        || crate::services::assistant_service_execution::is_shared_chat(chat)
    {
        return Ok(false);
    }
    if !crate::services::feature_flag_service::personal_flag_enabled(
        db,
        &chat.user_id,
        super::FLAG_KEY,
    )
    .await?
    {
        return Ok(false);
    }
    let agent = db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id": &chat.agent_id, "destroyed_at": mongodb::bson::Bson::Null})
        .await?;
    Ok(agent.is_some_and(|agent| chat.is_orchestrator() || !agent.grants.crm.is_empty()))
}

pub fn endpoints() -> Vec<McpToolEndpoint> {
    NAMES.iter().map(|name| McpToolEndpoint {
        endpoint_id: format!("nyxid__{name}"), name: (*name).into(), method: "POST".into(),
        description: Some(match *name {
            "crm_present" => "Show an existing saved view or record as a live artifact in the app and a link in chat apps. Pass kind and id; authority is checked live. No snapshot is stored in chat. When you use nyxid__post_to_chat, paste the returned NyxID workspace_url link into its text; that action does not attach turn artifacts automatically.",
            "crm_create_object" => "Create a custom CRM dataset and reusable live artifact. Requires Schema authority on the proposed immutable key. Initial typed fields and optional pipeline use the same validators as field editing. Artifacts render live in the app and as links in chat apps.",
            "crm_schema" => "List typed CRM objects, fields, required values, relationships and pipeline metadata. Use immutable field keys.",
            "crm_upsert_record" => "Match/create/update a CRM record by an explicit active text identity (trimmed, case-insensitive exact match). Ambiguous or archived matches fail. Input: object, match_field, values, optional expected_revision. Never guess relationships. Human overrides and fresher run results are preserved; inspect skipped fields. Reuse identity across scheduled runs.",
            "crm_query" => "Bounded CRM table query with filters, projection and cursor. Input uses the shared table query contract; follow next_cursor for more records.",
            "crm_aggregate" => "Current-record count/sum/avg/min/max. Group by category or date with date_bucket:{unit:day|week|month,timezone:IANA}. Calendar dates require UTC. No historical snapshots or inferred trends.",
            "crm_save_view" | "crm_update_view" => "Save a validated nyxid-crm-v1 json-render envelope. Use shared table/aggregate queries and closed CRM components. Save a view to give the person a reusable live artifact in the app and links in chat apps. Use crm_present to show an existing view or record. Saving does not invoke a model. Invalid bindings return actionable paths. When you use nyxid__post_to_chat, paste the returned NyxID workspace_url link into its text; that action does not attach turn artifacts automatically.",
            _ => "Read or mutate owner-scoped CRM data through the shared typed services. Writes require expected revisions where applicable. Use crm_schema for definitions; all authority and source identity are server derived.",
        }.into()),
        request_content_type: Some("application/json".into()), request_body_required: true,
        request_body_schema: Some(schema(name)),
        ..Default::default()
    }).collect()
}

fn input<T: serde::de::DeserializeOwned>(args: &Value) -> AppResult<T> {
    serde_json::from_value(args.get("input").cloned().unwrap_or(json!({})))
        .map_err(|error| super::validation_error("input", error.to_string()))
}
fn id<'a>(args: &'a Value, key: &str) -> AppResult<&'a str> {
    args[key]
        .as_str()
        .filter(|s| uuid::Uuid::parse_str(s).is_ok())
        .ok_or_else(|| super::validation_error(key, "must be a UUID"))
}
fn encoded<T: serde::Serialize>(value: T) -> AppResult<Value> {
    serde_json::to_value(value)
        .map_err(|_| AppError::Internal("CRM tool response could not be serialized".into()))
}

async fn confirmation_summary(
    db: &mongodb::Database,
    owner: &str,
    name: &str,
    args: &Value,
    object: Option<&str>,
) -> AppResult<String> {
    let action = name.strip_prefix("crm_").unwrap_or(name).replace('_', " ");
    let mut summary = format!("CRM {action}; owner {owner}");
    if let Some(object) = object {
        summary.push_str(&format!("; object {object}"));
    }
    if let Some(id) = args["record_id"].as_str() {
        let record = records::get(db, owner, id).await?;
        let metadata = objects::get(db, owner, &record.object_key).await?;
        let label = metadata
            .fields
            .iter()
            .find(|field| {
                field.required
                    && matches!(
                        field.kind,
                        crate::models::crm_object::CrmFieldKind::Text { .. }
                    )
            })
            .and_then(|field| record.values.get(&field.key))
            .and_then(Value::as_str)
            .unwrap_or(id);
        summary.push_str(&format!(
            "; record {} ({id}); /assistant/crm?owner={owner}&object={}&record={id}",
            bounded_label(label),
            record.object_key
        ));
    }
    if let Some(id) = args["view_id"].as_str() {
        let title = if name == "crm_delete_view" {
            views::for_delete(db, owner, id).await?.title
        } else {
            views::get(db, owner, id).await?.title
        };
        summary.push_str(&format!(
            "; view {} ({id}); /assistant/crm?owner={owner}&view={id}",
            bounded_label(&title)
        ));
    }
    if let Some(field) = args["field_key"]
        .as_str()
        .or_else(|| args["input"]["field"]["key"].as_str())
    {
        summary.push_str(&format!("; schema field {field}"));
        let change = if name == "crm_add_field" {
            &args["input"]["field"]
        } else {
            &args["input"]
        };
        for property in [
            "type",
            "label",
            "required",
            "archived",
            "target",
            "cardinality",
            "format",
            "currency",
        ] {
            if let Some(value) = change.get(property) {
                summary.push_str(&format!(
                    "; {property}={}",
                    bounded_label(&value.to_string())
                ));
            }
        }
        if let Some(options) = change["options"].as_array() {
            summary.push_str(&format!(
                "; options ({}): {}",
                options.len(),
                options
                    .iter()
                    .take(8)
                    .map(|option| format!(
                        "{}={}{}",
                        option["value"].as_str().unwrap_or_default(),
                        bounded_label(option["label"].as_str().unwrap_or_default()),
                        if option["archived"] == true {
                            " [archived]"
                        } else {
                            " [active]"
                        }
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            if options.len() > 8 {
                summary.push_str("; additional options changed");
            }
        }
    }
    if let Some(values) = args["input"]["values"].as_object() {
        let mut keys: Vec<_> = values.keys().cloned().collect();
        keys.sort();
        summary.push_str(&format!("; fields {}", keys.join(", ")));
    }
    Ok(summary.chars().take(1000).collect())
}
fn bounded_label(label: &str) -> String {
    label.chars().filter(|c| !c.is_control()).take(80).collect()
}

pub fn execute<'a>(
    state: &'a AppState,
    original: &'a acks::ChatAuthority,
    tool: &'a str,
    args: &'a Value,
    request_id: &'a Value,
) -> futures::future::BoxFuture<'a, AppResult<Value>> {
    Box::pin(execute_inner(state, original, tool, args, request_id))
}

async fn execute_inner(
    state: &AppState,
    original: &acks::ChatAuthority,
    tool: &str,
    args: &Value,
    _request_id: &Value,
) -> AppResult<Value> {
    let name = tool.strip_prefix("nyxid__").unwrap_or(tool);
    let class =
        action_class(name).ok_or_else(|| AppError::NotFound("CRM tool not found".into()))?;
    if args.to_string().len() > 65_536 || !validate_shape(args, &schema(name)) {
        return Err(super::validation_error(
            "arguments",
            "invalid arguments; follow this tool's typed schema and omit irrelevant keys",
        ));
    }
    let chat =
        crate::services::assistant_service_execution::live_authority(&state.db, original, tool)
            .await?;
    chat.ensure_live_turn(&state.db, None, None, tool).await?;
    if !available(&state.db, &chat).await? {
        return Err(AppError::Forbidden(
            "CRM access is not granted for this conversation".into(),
        ));
    }
    let mut agent = state
        .db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id": &chat.agent_id})
        .await?
        .ok_or_else(|| AppError::Forbidden("Agent unavailable".into()))?;
    let owner_id = args["owner_id"]
        .as_str()
        .unwrap_or(&agent.user_id)
        .to_owned();
    let owner = owner_id.as_str();
    if !chat.is_orchestrator() && owner != agent.user_id {
        return Err(AppError::Forbidden(
            "CRM grant is limited to the specialist's owner".into(),
        ));
    }
    let access =
        crate::services::org_service::resolve_owner_access(&state.db, &chat.user_id, owner).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("CRM owner not found".into()));
    }
    if class != acks::OwnerActionClass::Read
        && !(access.can_write()
            || matches!(&access, crate::services::org_service::OwnerAccess::AsOrgMember { role, .. } if role.can_proxy()))
    {
        return Err(AppError::Forbidden(
            "CRM record writes require owner or member access".into(),
        ));
    }
    objects::ensure_seeded(&state.db, owner).await?;
    let capability = match name {
        "crm_add_field" | "crm_update_field" | "crm_create_object" => CrmCapability::Schema,
        _ if class == acks::OwnerActionClass::Read => CrmCapability::Read,
        _ => CrmCapability::Write,
    };
    let object_key = if matches!(
        name,
        "crm_record"
            | "crm_patch_record"
            | "crm_archive_record"
            | "crm_restore_record"
            | "crm_add_note"
            | "crm_activities"
    ) {
        let id = id(args, "record_id")?;
        Some(records::get(&state.db, owner, id).await?.object_key)
    } else if name == "crm_present" && args["kind"] == "record" {
        Some(
            records::get(&state.db, owner, id(args, "id")?)
                .await?
                .object_key,
        )
    } else if name == "crm_create_object" {
        args["input"]["key"].as_str().map(str::to_owned)
    } else {
        args["input"]["object"]
            .as_str()
            .or_else(|| args["object"].as_str())
            .map(str::to_owned)
    };
    if capability == CrmCapability::Schema && !access.can_write() {
        return Err(AppError::Forbidden(
            "Schema edits require owner or organization administrator access".into(),
        ));
    }
    authorize_operation(
        &state.db,
        &chat,
        &agent,
        owner,
        name,
        args,
        object_key.as_deref(),
        capability,
    )
    .await?;
    let receipt = if class == acks::OwnerActionClass::Read {
        None
    } else {
        let mut payload = args.clone();
        if let Some(map) = payload.as_object_mut() {
            map.remove("acknowledgement_id");
        }
        if args["mutation_key"]
            .as_str()
            .is_some_and(|key| !key.bytes().all(|c| c.is_ascii_graphic()))
        {
            return Err(super::validation_error(
                "mutation_key",
                "use visible ASCII without spaces",
            ));
        }
        let key = mutation_identity(
            &chat.conversation_id,
            args["mutation_key"].as_str().unwrap_or_default(),
        );
        records::receipt_context(
            owner,
            &chat.user_id,
            Some(&key),
            "TOOL",
            tool,
            &[],
            &payload,
        )?
    };
    // Confirmation cards are single-use. An exact committed retry recovers its
    // result without consuming the card again, while still passing every live
    // authority check below. Retain the result so TTL cleanup cannot turn this
    // recovery into a new effect between receipt lookups.
    let committed = records::lookup_receipt::<Value>(&state.db, receipt.as_ref()).await?;
    let gate =
        acks::confirmation_needed(&state.db, &chat, acks::ActionFacts::of(class), false).await?;
    if let Some(need) = gate.ask().filter(|_| committed.is_none()) {
        if let Some(refusal) = acks::confirm_action(
            &state.db,
            &chat,
            tool,
            args,
            &need,
            &confirmation_summary(&state.db, owner, name, args, object_key.as_deref()).await?,
        )
        .await?
        {
            return Ok(refusal);
        }
    }
    // Reload after confirmation before touching data. An acknowledgement never
    // restores a revoked grant, stopped turn, or removed organization member.
    let chat = crate::services::assistant_service_execution::live_authority(&state.db, &chat, tool)
        .await?;
    if !available(&state.db, &chat).await? {
        return Err(AppError::Forbidden("CRM grant was revoked".into()));
    }
    agent = state
        .db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id": &chat.agent_id, "destroyed_at": mongodb::bson::Bson::Null})
        .await?
        .ok_or_else(|| AppError::Forbidden("Agent unavailable".into()))?;
    let access =
        crate::services::org_service::resolve_owner_access(&state.db, &chat.user_id, owner).await?;
    let work = access.can_write()
        || matches!(&access, crate::services::org_service::OwnerAccess::AsOrgMember { role, .. } if role.can_proxy());
    if !access.can_read()
        || (class != acks::OwnerActionClass::Read && !work)
        || (capability == CrmCapability::Schema && !access.can_write())
    {
        return Err(AppError::Forbidden("CRM owner authority changed".into()));
    }
    authorize_operation(
        &state.db,
        &chat,
        &agent,
        owner,
        name,
        args,
        object_key.as_deref(),
        capability,
    )
    .await?;
    if name == "crm_delete_view" {
        let view = views::for_delete(&state.db, owner, id(args, "view_id")?).await?;
        if view.created_by.user_id != chat.user_id && !access.can_write() {
            return Err(AppError::Forbidden(
                "Deleting another person's view requires schema authority".into(),
            ));
        }
    }
    let conversation = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id": &chat.conversation_id})
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    let turn = crate::services::assistant_nyxagent::live_turn(&conversation, chrono::Utc::now())
        .filter(|turn| Some(&turn.turn_id) == chat.turn_id.as_ref() && !turn.stop_requested)
        .cloned()
        .ok_or(AppError::AssistantTurnRequired)?;
    let mut ordered_at = turn.started_at;
    if let Some(run_id) = &turn.trigger_run_id {
        let run = state
            .db
            .collection::<crate::models::trigger_run::TriggerRun>(
                crate::models::trigger_run::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": run_id})
            .await?
            .ok_or_else(|| AppError::Forbidden("Scheduled run unavailable".into()))?;
        ordered_at = run.scheduled_at;
    }
    let actor = CrmActor {
        kind: CrmActorKind::Agent,
        user_id: chat.user_id.clone(),
        agent_id: Some(chat.agent_id.clone()),
        api_key_id: Some(chat.api_key_id.clone()),
        source: Some(CrmSource {
            conversation_id: chat.conversation_id.clone(),
            turn_id: turn.turn_id.clone(),
            run_id: turn.trigger_run_id,
            ordered_at,
        }),
    };
    let admission = MutationAdmission {
        user_id: chat.user_id.clone(),
        agent_id: agent.id.clone(),
        agent_owner_id: agent.user_id.clone(),
        session_id: conversation.nyxagent_session_id.clone(),
        conversation_id: chat.conversation_id.clone(),
        turn_id: turn.turn_id.clone(),
        api_key_id: chat.api_key_id.clone(),
        grants: mongodb::bson::to_bson(&agent.grants.crm)
            .map_err(|_| AppError::Internal("CRM authority serialization failed".into()))?,
        owner: owner.into(),
        schema: capability == CrmCapability::Schema
            || (name == "crm_delete_view" && access.can_write()),
    };
    let operation = async {
        match name {
            "crm_present" => {
                if args["kind"] == "view" {
                    let view = views::get(&state.db, owner, id(args, "id")?).await?;
                    Ok(view_summary(&view))
                } else {
                    let record = records::get(&state.db, owner, id(args, "id")?).await?;
                    Ok(record_summary(&state.db, owner, &record).await?)
                }
            }
            "crm_create_object" => encoded(objects::ObjectApi::from(
                objects::create(&state.db, owner, input(args)?, receipt).await?,
            )),
            "crm_schema" => encoded(
                objects::list(&state.db, owner)
                    .await?
                    .into_iter()
                    .filter(|object| {
                        require_object(&chat, &agent, owner, &object.key, CrmCapability::Read)
                            .is_ok()
                    })
                    .map(objects::ObjectApi::from)
                    .collect::<Vec<_>>(),
            ),
            "crm_query" => encoded(query::table(&state.db, owner, &input(args)?).await?),
            "crm_aggregate" => encoded(query::aggregate(&state.db, owner, &input(args)?).await?),
            "crm_record" => encoded(records::get(&state.db, owner, id(args, "record_id")?).await?),
            "crm_upsert_record" => {
                encoded(transfer::upsert(&state.db, owner, actor, input(args)?, receipt).await?)
            }
            "crm_patch_record" => encoded(
                records::patch(
                    &state.db,
                    owner,
                    id(args, "record_id")?,
                    actor,
                    input(args)?,
                    receipt,
                )
                .await?,
            ),
            "crm_archive_record" | "crm_restore_record" => encoded(
                records::set_archived(
                    &state.db,
                    owner,
                    id(args, "record_id")?,
                    actor,
                    input(args)?,
                    name == "crm_archive_record",
                    receipt,
                )
                .await?,
            ),
            "crm_add_note" => encoded(
                activities::add_note(
                    &state.db,
                    owner,
                    id(args, "record_id")?,
                    actor,
                    input(args)?,
                    receipt,
                )
                .await?,
            ),
            "crm_activities" => encoded(
                activities::list(
                    &state.db,
                    owner,
                    id(args, "record_id")?,
                    args["cursor"].as_str(),
                )
                .await?,
            ),
            "crm_views" => {
                let mut page = views::list_page(&state.db, owner, args["cursor"].as_str()).await?;
                let metadata = objects::list(&state.db, owner).await?;
                page.views.retain(|view| {
                    authorize_envelope(
                        &chat,
                        &agent,
                        owner,
                        &view.envelope,
                        CrmCapability::Read,
                        &metadata,
                    )
                    .is_ok()
                });
                encoded(page)
            }
            "crm_view" => encoded(views::get(&state.db, owner, id(args, "view_id")?).await?),
            "crm_save_view" => {
                encoded(views::create(&state.db, owner, actor, input(args)?, receipt).await?)
            }
            "crm_update_view" => encoded(
                views::update(
                    &state.db,
                    owner,
                    id(args, "view_id")?,
                    input(args)?,
                    receipt,
                )
                .await?,
            ),
            "crm_delete_view" => {
                encoded(views::delete(&state.db, owner, id(args, "view_id")?, receipt).await?)
            }
            "crm_add_field" => {
                let body: AddField = input(args)?;
                encoded(objects::ObjectApi::from(
                    objects::add_field(
                        &state.db,
                        owner,
                        args["object"].as_str().unwrap_or_default(),
                        body.expected_schema_revision,
                        body.field,
                        receipt,
                    )
                    .await?,
                ))
            }
            "crm_update_field" => {
                let body: objects::PatchField = input(args)?;
                encoded(objects::ObjectApi::from(
                    objects::patch_field(
                        &state.db,
                        owner,
                        args["object"].as_str().unwrap_or_default(),
                        args["field_key"].as_str().unwrap_or_default(),
                        body,
                        receipt,
                    )
                    .await?,
                ))
            }
            _ => unreachable!(),
        }
    };
    let operation: AppResult<Value> = if let Some(response) = committed {
        if matches!(
            name,
            "crm_add_field" | "crm_update_field" | "crm_create_object"
        ) {
            // Schema services persist their model response; native tools expose
            // the same API DTO on first execution and receipt recovery.
            let object = serde_json::from_value::<crate::models::crm_object::CrmObject>(response)
                .map_err(|_| {
                AppError::Internal("Stored CRM schema result is invalid".into())
            })?;
            encoded(objects::ObjectApi::from(object))
        } else {
            Ok(response)
        }
    } else if class == acks::OwnerActionClass::Read {
        operation.await
    } else {
        MUTATION_ADMISSION.scope(admission, operation).await
    };
    let mut result = match operation {
        Ok(value) => value,
        Err(mut error) => {
            if let AppError::CrmRevisionConflict(current) = &mut error {
                redact_links(&chat, &agent, owner, current);
            }
            return Err(error);
        }
    };

    redact_links(&chat, &agent, owner, &mut result);
    use crate::models::assistant_crm_artifact::{CrmArtifactKind, CrmArtifactRole};
    let artifact = match name {
        "crm_save_view" | "crm_update_view" => {
            Some((CrmArtifactKind::View, CrmArtifactRole::Created))
        }
        "crm_create_object" => Some((CrmArtifactKind::Object, CrmArtifactRole::Created)),
        "crm_upsert_record" if result["created"] == true => {
            Some((CrmArtifactKind::Record, CrmArtifactRole::Created))
        }
        "crm_present" => Some((
            if args["kind"] == "view" {
                CrmArtifactKind::View
            } else {
                CrmArtifactKind::Record
            },
            CrmArtifactRole::Presented,
        )),
        _ => None,
    };
    if let Some((kind, role)) = artifact {
        let reference = artifact_reference(&state.db, owner, kind, role, &result).await;
        result = super::artifacts::committed_result(&state.db, &chat, reference, result).await;
    }

    if class != acks::OwnerActionClass::Read {
        let _ = crate::services::audit_service::log_actor_event(state.db.clone(), &crate::services::audit_service::AuditActor { user_id: chat.user_id.clone(), api_key_id: Some(chat.api_key_id.clone()), api_key_name: None, ip_address: None, user_agent: None }, "crm_tool_mutation", Some(json!({"owner_id":owner,"tool":name,"conversation_id":chat.conversation_id,"turn_id":turn.turn_id}))).await;
    }
    if let Some(auto) = gate.auto_allowed() {
        acks::record_auto_allowed(&state.db, &chat, tool, args, &auto, "CRM operation").await;
    }
    Ok(
        json!({"result":result,"owner_id":owner,"workspace_url":format!("{}/assistant/crm?owner={}",state.config.frontend_url.trim_end_matches('/'),owner)}),
    )
}

async fn record_summary(
    db: &mongodb::Database,
    owner: &str,
    record: &records::RecordResponse,
) -> AppResult<Value> {
    let object = objects::get(db, owner, &record.object_key).await?;
    let title = record_title(&object, &record.values);
    let fields: Vec<_> = object.fields.iter().filter(|field| !field.archived)
        .filter_map(|field| {
            let value = record.values.get(&field.key)?;
            let value = match value {
                Value::String(value) => json!(super::artifacts::title(value)),
                Value::Bool(_) | Value::Number(_) | Value::Null => value.clone(),
                _ => return None,
            };
            Some(json!({"key":field.key,"label":super::artifacts::title(&field.label),"value":value}))
        }).take(5).collect();
    Ok(
        json!({"id":record.id,"kind":"record","object_key":record.object_key,"object_label":super::artifacts::title(&object.label),"title":title,"fields":fields,"revision":record.revision}),
    )
}

fn record_title(
    object: &crate::models::crm_object::CrmObject,
    values: &std::collections::BTreeMap<String, Value>,
) -> String {
    let field = object.fields.iter().find(|field| {
        field.required
            && matches!(
                field.kind,
                crate::models::crm_object::CrmFieldKind::Text { .. }
            )
    });
    super::artifacts::title(
        field
            .and_then(|field| values.get(&field.key))
            .and_then(Value::as_str)
            .unwrap_or(&object.label),
    )
}

fn view_summary(view: &views::ViewResponse) -> Value {
    let objects: std::collections::BTreeSet<_> = view.envelope["queries"]
        .as_object()
        .into_iter()
        .flat_map(|queries| queries.values())
        .filter_map(|query| query["query"]["object"].as_str())
        .take(12)
        .collect();
    let components: std::collections::BTreeSet<_> = view.envelope["spec"]["elements"]
        .as_object()
        .into_iter()
        .flat_map(|elements| elements.values())
        .filter_map(|element| element["type"].as_str())
        .take(60)
        .collect();
    json!({"id":view.id,"kind":"view","title":super::artifacts::title(&view.title),"objects":objects,"components":components})
}

async fn artifact_reference(
    db: &mongodb::Database,
    owner: &str,
    kind: crate::models::assistant_crm_artifact::CrmArtifactKind,
    role: crate::models::assistant_crm_artifact::CrmArtifactRole,
    result: &Value,
) -> AppResult<crate::models::assistant_crm_artifact::TurnCrmArtifact> {
    use crate::models::assistant_crm_artifact::{CrmArtifactKind, TurnCrmArtifact};
    let id = result["id"]
        .as_str()
        .ok_or_else(|| AppError::Internal("CRM artifact identity missing".into()))?
        .to_owned();
    let owner_id = owner.to_owned();
    let title = super::artifacts::title(
        result["title"]
            .as_str()
            .or_else(|| result["label"].as_str())
            .unwrap_or("Saved view"),
    );
    if kind == CrmArtifactKind::View {
        return Ok(TurnCrmArtifact::View {
            id,
            owner_id,
            title,
            role,
        });
    }
    let object_key = result["object_key"]
        .as_str()
        .or_else(|| result["key"].as_str())
        .ok_or_else(|| AppError::Internal("CRM artifact object key missing".into()))?
        .to_owned();
    if kind == CrmArtifactKind::Object {
        return Ok(TurnCrmArtifact::Object {
            id,
            owner_id,
            object_key,
            title,
            role,
        });
    }
    let title = if result["title"].is_string() {
        title
    } else {
        let object = objects::get(db, owner, &object_key).await?;
        let values = serde_json::from_value(result["values"].clone())
            .map_err(|_| AppError::Internal("CRM artifact record values missing".into()))?;
        record_title(&object, &values)
    };
    Ok(TurnCrmArtifact::Record {
        id,
        owner_id,
        object_key,
        title,
        role,
    })
}

use crate::models::assistant_agent::{CrmCapability, CrmGrant};
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AddField {
    expected_schema_revision: i64,
    field: crate::models::crm_object::CrmField,
}

pub async fn validate_grants(
    db: &mongodb::Database,
    owner: &str,
    grants: &[CrmGrant],
) -> AppResult<()> {
    if grants.len() > 16 {
        return Err(super::validation_error(
            "crm",
            "at most 16 object grants are allowed",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for grant in grants {
        if grant.owner_id != owner
            || !seen.insert(&grant.object)
            || grant.capabilities.is_empty()
            || grant.capabilities.len() > 3
        {
            return Err(super::validation_error(
                "crm",
                "grants require the agent owner, unique objects and explicit capabilities",
            ));
        }
        if !grant.capabilities.contains(&CrmCapability::Read) {
            return Err(super::validation_error(
                "crm",
                "write/schema grants must also include read",
            ));
        }
        if !super::fields::valid_key(&grant.object) {
            return Err(super::validation_error(
                "crm.object",
                "must be an immutable object key",
            ));
        }
        match objects::get(db, owner, &grant.object).await {
            Ok(_) => {}
            Err(AppError::NotFound(_))
                if grant.capabilities.contains(&CrmCapability::Schema)
                    && !objects::RESERVED_OBJECT_KEYS.contains(&grant.object.as_str()) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn require_object(
    chat: &acks::ChatAuthority,
    agent: &AssistantAgent,
    owner: &str,
    object: &str,
    capability: CrmCapability,
) -> AppResult<()> {
    if chat.is_orchestrator()
        || agent.grants.crm.iter().any(|grant| {
            grant.owner_id == owner
                && grant.object == object
                && grant.capabilities.contains(&capability)
        })
    {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "CRM {capability:?} access is not granted for this object"
        )))
    }
}
async fn authorize_operation(
    db: &mongodb::Database,
    chat: &acks::ChatAuthority,
    agent: &AssistantAgent,
    owner: &str,
    name: &str,
    args: &Value,
    object_key: Option<&str>,
    capability: CrmCapability,
) -> AppResult<()> {
    if let Some(key) = object_key {
        require_object(chat, agent, owner, key, capability)?;
        if name != "crm_create_object" {
            let object = objects::get(db, owner, key).await?;
            authorize_references(chat, agent, owner, &object, &args["input"])?;
        }
    }
    if name == "crm_create_object" {
        let body: objects::CreateObjectInput = input(args)?;
        for field in body.fields {
            if let crate::models::crm_object::CrmFieldKind::Record { target, .. } = field.kind {
                if Some(target.as_str()) != object_key {
                    require_object(chat, agent, owner, &target, CrmCapability::Read)?;
                }
            }
        }
    }
    if matches!(name, "crm_add_field") {
        if args["input"]["field"]["type"] == "record" {
            require_object(
                chat,
                agent,
                owner,
                args["input"]["field"]["target"]
                    .as_str()
                    .unwrap_or_default(),
                CrmCapability::Read,
            )?;
        }
    }
    if name == "crm_present" && args["kind"] == "view" {
        let view = views::get(db, owner, id(args, "id")?).await?;
        let metadata = objects::list(db, owner).await?;
        authorize_envelope(
            chat,
            agent,
            owner,
            &view.envelope,
            CrmCapability::Read,
            &metadata,
        )?;
    }
    if args["input"].get("envelope").is_some() || args.get("view_id").is_some() {
        let metadata = objects::list(db, owner).await?;
        if let Some(envelope) = args["input"].get("envelope") {
            authorize_envelope(chat, agent, owner, envelope, capability, &metadata)?;
        }
        if let Some(view_id) = args["view_id"].as_str() {
            let envelope = if name == "crm_delete_view" {
                views::for_delete(db, owner, view_id).await?.envelope
            } else {
                views::get(db, owner, view_id).await?.envelope
            };
            authorize_envelope(chat, agent, owner, &envelope, capability, &metadata)?;
        }
    }
    Ok(())
}
fn authorize_references(
    chat: &acks::ChatAuthority,
    agent: &AssistantAgent,
    owner: &str,
    object: &crate::models::crm_object::CrmObject,
    input: &Value,
) -> AppResult<()> {
    for field in &object.fields {
        if let crate::models::crm_object::CrmFieldKind::Record { target, .. } = &field.kind {
            let used = input["values"].get(&field.key).is_some()
                || input["filters"].as_array().is_some_and(|filters| {
                    filters.iter().any(|filter| filter["field"] == field.key)
                })
                || input["group_by"] == field.key;
            if used {
                require_object(chat, agent, owner, target, CrmCapability::Read)?;
            }
        }
    }
    Ok(())
}
fn authorize_envelope(
    chat: &acks::ChatAuthority,
    agent: &AssistantAgent,
    owner: &str,
    envelope: &Value,
    capability: CrmCapability,
    metadata: &[crate::models::crm_object::CrmObject],
) -> AppResult<()> {
    let mut referenced = false;
    if let Some(queries) = envelope["queries"].as_object() {
        for query in queries.values() {
            let object = query["query"]["object"]
                .as_str()
                .ok_or_else(|| super::validation_error("envelope", "query object is required"))?;
            referenced = true;
            require_object(chat, agent, owner, object, capability)?;
            if let Some(definition) = metadata.iter().find(|entry| entry.key == object) {
                authorize_references(chat, agent, owner, definition, &query["query"])?;
            }
        }
    }
    if let Some(elements) = envelope["spec"]["elements"].as_object() {
        for element in elements.values() {
            if element["type"] == "RecordForm" {
                referenced = true;
                let object = element["props"]["object"].as_str().unwrap_or_default();
                if let Some(definition) = metadata.iter().find(|entry| entry.key == object) {
                    for field in &definition.fields {
                        if let crate::models::crm_object::CrmFieldKind::Record { target, .. } =
                            &field.kind
                        {
                            let included = element["props"]["fields"]
                                .as_array()
                                .is_none_or(|fields| fields.iter().any(|key| key == &field.key));
                            if included {
                                require_object(chat, agent, owner, target, CrmCapability::Read)?;
                            }
                        }
                    }
                }
                require_object(
                    chat,
                    agent,
                    owner,
                    element["props"]["object"].as_str().unwrap_or_default(),
                    CrmCapability::Write,
                )?;
            }
        }
    }
    if !referenced && !chat.is_orchestrator() {
        return Err(AppError::Forbidden(
            "Specialist saved views must reference an explicitly granted CRM object".into(),
        ));
    }
    Ok(())
}
fn redact_links(
    chat: &acks::ChatAuthority,
    agent: &AssistantAgent,
    owner: &str,
    value: &mut Value,
) {
    if let Some(map) = value.as_object_mut() {
        if let Some(links) = map.get_mut("links").and_then(Value::as_object_mut) {
            for entries in links.values_mut() {
                if let Some(entries) = entries.as_array_mut() {
                    entries.retain(|link| {
                        link["object_key"].as_str().is_some_and(|object| {
                            require_object(chat, agent, owner, object, CrmCapability::Read).is_ok()
                        })
                    });
                }
            }
        }
        for child in map.values_mut() {
            redact_links(chat, agent, owner, child);
        }
    } else if let Some(items) = value.as_array_mut() {
        for item in items {
            redact_links(chat, agent, owner, item);
        }
    }
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn key_schema() -> Value {
    json!({"type":"string","pattern":"^[a-z][a-z0-9_]{0,47}$","maxLength":48})
}
pub fn schema(name: &str) -> Value {
    if name == "crm_present" {
        return object_schema(
            json!({"kind":{"enum":["view","record"]},"id":{"type":"string","format":"uuid"},"owner_id":{"type":"string","format":"uuid"}}),
            &["kind", "id"],
        );
    }

    let key = key_schema();
    let filters = json!({"type":"array","maxItems":20,"items":object_schema(json!({"field":key,"op":{"type":"string","enum":["eq","neq","in","nin","contains","gt","gte","lt","lte","is_set","is_unset"]},"value":{}}), &["field","op"])});
    let values = json!({"type":"object","maxProperties":100,"description":"Stable field keys to typed values. Currency is integer hundredths; percent is a fraction; relation values are UUID/string arrays; null clears optional values."});
    let revision = json!({"type":"integer","minimum":1});
    let table = object_schema(
        json!({"object":key,"select":{"type":"array","maxItems":100,"items":key},"filters":filters,"sort":object_schema(json!({"field":key,"direction":{"type":"string","enum":["asc","desc"]}}), &["field","direction"]),"page_size":{"type":"integer","minimum":1,"maximum":200},"cursor":{"type":"string","minLength":1,"maxLength":4096},"include_archived":{"type":"boolean"}}),
        &["object"],
    );
    let aggregate = object_schema(
        json!({"object":key,"filters":filters,"group_by":key,"date_bucket":object_schema(json!({"unit":{"type":"string","enum":["day","week","month"]},"timezone":{"type":"string","minLength":1,"maxLength":100,"description":"IANA timezone; calendar dates require UTC."}}), &["unit","timezone"]),"measure":{"oneOf":[object_schema(json!({"op":{"const":"count"}}), &["op"]),object_schema(json!({"op":{"type":"string","enum":["sum","avg","min","max"]},"field":key}), &["op","field"])]},"order":{"type":"string","enum":["option_order","value_desc","label_asc"]}}),
        &["object", "measure", "order"],
    );
    let option = object_schema(
        json!({"value":{"type":"string","pattern":"^[a-z0-9_]{1,48}$","maxLength":48},"label":{"type":"string","minLength":1,"maxLength":200},"color":{"type":["string","null"],"maxLength":64},"archived":{"type":"boolean"}}),
        &["value", "label"],
    );
    let mut field_variants = Vec::new();
    for (kind, extras, required) in [
        (
            "text",
            json!({"max_length":{"type":"integer","minimum":1,"maximum":10000}}),
            vec![],
        ),
        (
            "number",
            json!({"format":{"type":"string","enum":["plain","percent"]}}),
            vec!["format"],
        ),
        (
            "currency",
            json!({"currency":{"type":"string","enum":["USD","EUR","GBP","SGD","AUD","CAD","HKD","CNY"]}}),
            vec!["currency"],
        ),
        ("boolean", json!({}), vec![]),
        ("date", json!({}), vec![]),
        ("datetime", json!({}), vec![]),
        ("user", json!({}), vec![]),
        (
            "select",
            json!({"options":{"type":"array","maxItems":100,"items":option}}),
            vec!["options"],
        ),
        (
            "record",
            json!({"target":key,"cardinality":{"type":"string","enum":["one","many"]}}),
            vec!["target", "cardinality"],
        ),
    ] {
        let mut properties = json!({"key":key,"label":{"type":"string","minLength":1,"maxLength":200},"required":{"type":"boolean"},"archived":{"type":"boolean"},"type":{"const":kind}});
        properties
            .as_object_mut()
            .unwrap()
            .extend(extras.as_object().unwrap().clone());
        let mut keys = vec!["key", "label", "type", "required", "archived"];
        keys.extend(required);
        field_variants.push(object_schema(properties, &keys));
    }
    let identifier =
        json!({"type":"string","pattern":"^[A-Za-z][A-Za-z0-9_-]{0,63}$","maxLength":64});
    let mut elements = Vec::new();
    for (kind, props, required, container) in [
        (
            "Stack",
            json!({"gap":{"type":"string","enum":["sm","md","lg"]},"direction":{"type":"string","enum":["vertical","horizontal"]}}),
            vec!["gap"],
            true,
        ),
        (
            "Grid",
            json!({"columns":{"type":"integer","minimum":1,"maximum":4}}),
            vec!["columns"],
            true,
        ),
        (
            "Text",
            json!({"content":{"type":"string","minLength":1,"maxLength":2000},"variant":{"type":"string","enum":["title","heading","body","muted"]}}),
            vec!["content", "variant"],
            false,
        ),
        (
            "Metric",
            json!({"queryId":identifier,"title":{"type":"string","maxLength":200}}),
            vec!["queryId", "title"],
            false,
        ),
        (
            "BarChart",
            json!({"queryId":identifier,"title":{"type":"string","maxLength":200},"height":{"type":"string","enum":["small","medium","large"]}}),
            vec!["queryId", "title"],
            false,
        ),
        (
            "PieChart",
            json!({"queryId":identifier,"title":{"type":"string","maxLength":200},"height":{"type":"string","enum":["small","medium","large"]}}),
            vec!["queryId", "title"],
            false,
        ),
        (
            "ObjectTable",
            json!({"queryId":identifier,"columns":{"type":"array","maxItems":100,"items":key}}),
            vec!["queryId"],
            false,
        ),
        (
            "PipelineBoard",
            json!({"queryId":identifier}),
            vec!["queryId"],
            false,
        ),
        (
            "RecordForm",
            json!({"object":key,"fields":{"type":"array","maxItems":100,"items":key},"title":{"type":"string","maxLength":200},"submitLabel":{"type":"string","maxLength":200}}),
            vec!["object", "title"],
            false,
        ),
    ] {
        elements.push(object_schema(json!({"type":{"const":kind},"props":object_schema(props,&required),"children":{"type":"array","maxItems":if container {60} else {0},"items":identifier}}), &["type","props","children"]));
    }
    let envelope = object_schema(
        json!({"formatVersion":{"const":1},"renderer":{"const":"json-render"},"rendererSpecVersion":{"const":"nyxid-crm-v1"},"queries":{"type":"object","maxProperties":12,"propertyNames":identifier,"additionalProperties":{"oneOf":[object_schema(json!({"kind":{"const":"table"},"query":table}), &["kind","query"]),object_schema(json!({"kind":{"const":"aggregate"},"query":aggregate}), &["kind","query"])]}},"spec":object_schema(json!({"root":identifier,"elements":{"type":"object","maxProperties":60,"propertyNames":identifier,"additionalProperties":{"oneOf":elements}}}), &["root","elements"])}),
        &[
            "formatVersion",
            "renderer",
            "rendererSpecVersion",
            "queries",
            "spec",
        ],
    );
    let input = match name {
        "crm_create_object" => object_schema(
            json!({"key":key,"label":{"type":"string","minLength":1,"maxLength":200},"plural_label":{"type":"string","minLength":1,"maxLength":200},"fields":{"type":"array","maxItems":100,"items":{"oneOf":field_variants}},"pipeline":object_schema(json!({"stage_field":key,"closed_stages":{"type":"array","maxItems":50,"items":{"type":"string","pattern":"^[a-z0-9_]{1,48}$","maxLength":48}}}), &["stage_field","closed_stages"])}),
            &["key", "label", "plural_label", "fields"],
        ),
        "crm_query" => table,
        "crm_aggregate" => aggregate,
        "crm_upsert_record" => object_schema(
            json!({"object":key,"match_field":key,"values":values,"expected_revision":revision}),
            &["object", "match_field", "values"],
        ),
        "crm_patch_record" => object_schema(
            json!({"expected_revision":revision,"values":values}),
            &["expected_revision", "values"],
        ),
        "crm_archive_record" | "crm_restore_record" => object_schema(
            json!({"expected_revision":revision}),
            &["expected_revision"],
        ),
        "crm_add_note" => object_schema(
            json!({"body":{"type":"string","minLength":1,"maxLength":10000}}),
            &["body"],
        ),
        "crm_save_view" => object_schema(
            json!({"title":{"type":"string","minLength":1,"maxLength":200},"envelope":envelope}),
            &["title", "envelope"],
        ),
        "crm_update_view" => object_schema(
            json!({"title":{"type":"string","minLength":1,"maxLength":200},"envelope":envelope,"expected_revision":revision}),
            &["title", "envelope", "expected_revision"],
        ),
        "crm_add_field" => object_schema(
            json!({"expected_schema_revision":revision,"field":{"oneOf":field_variants}}),
            &["expected_schema_revision", "field"],
        ),
        "crm_update_field" => object_schema(
            json!({"expected_schema_revision":revision,"label":{"type":"string","minLength":1,"maxLength":200},"required":{"type":"boolean"},"archived":{"type":"boolean"},"options":{"type":"array","maxItems":100,"items":option}}),
            &["expected_schema_revision"],
        ),
        _ => Value::Null,
    };
    let mut properties = json!({"owner_id":{"type":"string","format":"uuid"},"acknowledgement_id":{"type":"string","format":"uuid"}});
    let mut required = Vec::new();
    if action_class(name).is_some_and(|class| class != acks::OwnerActionClass::Read) {
        properties["mutation_key"] = json!({"type":"string","minLength":1,"maxLength":128,"description":"Logical write identity. Reuse exactly with unchanged arguments after an uncertain response, even when transport request ID changes. Use a new key for distinct writes or changed revision/payload. Never reuse a previous scheduled run key for new enrichment."});
        required.push("mutation_key");
    }
    if !input.is_null() {
        properties["input"] = input;
        required.push("input");
    }
    if matches!(
        name,
        "crm_record"
            | "crm_patch_record"
            | "crm_archive_record"
            | "crm_restore_record"
            | "crm_add_note"
            | "crm_activities"
    ) {
        properties["record_id"] = json!({"type":"string","format":"uuid"});
        required.push("record_id");
    }
    if matches!(name, "crm_view" | "crm_update_view" | "crm_delete_view") {
        properties["view_id"] = json!({"type":"string","format":"uuid"});
        required.push("view_id");
    }
    if matches!(name, "crm_add_field" | "crm_update_field") {
        properties["object"] = key.clone();
        required.push("object");
    }
    if name == "crm_update_field" {
        properties["field_key"] = key;
        required.push("field_key");
    }
    if matches!(name, "crm_activities" | "crm_views") {
        properties["cursor"] = json!({"type":"string","maxLength":4096});
    }
    object_schema(properties, &required)
}
fn validate_shape(value: &Value, schema: &Value) -> bool {
    if let Some(options) = schema["oneOf"].as_array() {
        return options.iter().filter(|s| validate_shape(value, s)).count() == 1;
    }
    if schema
        .get("const")
        .is_some_and(|expected| expected != value)
        || schema["enum"]
            .as_array()
            .is_some_and(|a| !a.contains(value))
    {
        return false;
    }
    let matches = |kind: &str| match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    };
    if let Some(kind) = schema["type"].as_str() {
        if !matches(kind) {
            return false;
        }
    } else if let Some(kinds) = schema["type"].as_array() {
        if !kinds.iter().any(|k| k.as_str().is_some_and(matches)) {
            return false;
        }
    }
    if let Some(text) = value.as_str() {
        if schema["pattern"].as_str().is_some_and(|pattern| {
            regex::Regex::new(pattern).map_or(true, |pattern| !pattern.is_match(text))
        }) {
            return false;
        }
        if schema["maxLength"]
            .as_u64()
            .is_some_and(|max| text.chars().count() > max as usize)
            || schema["minLength"]
                .as_u64()
                .is_some_and(|min| text.chars().count() < min as usize)
            || (schema["format"] == "uuid" && uuid::Uuid::parse_str(text).is_err())
        {
            return false;
        }
    }
    if let Some(number) = value.as_i64() {
        if schema["minimum"].as_i64().is_some_and(|min| number < min)
            || schema["maximum"].as_i64().is_some_and(|max| number > max)
        {
            return false;
        }
    }
    if let Some(map) = value.as_object() {
        if schema["maxProperties"]
            .as_u64()
            .is_some_and(|max| map.len() > max as usize)
        {
            return false;
        }
        if schema["required"].as_array().is_some_and(|keys| {
            keys.iter()
                .any(|k| !map.contains_key(k.as_str().unwrap_or_default()))
        }) {
            return false;
        }
        for (key, value) in map {
            if let Some(prop) = schema["properties"].get(key) {
                if !validate_shape(value, prop) {
                    return false;
                }
            } else if schema["additionalProperties"] == false {
                return false;
            } else if schema["additionalProperties"].is_object()
                && !validate_shape(value, &schema["additionalProperties"])
            {
                return false;
            }
            if schema["propertyNames"].is_object()
                && !validate_shape(&json!(key), &schema["propertyNames"])
            {
                return false;
            }
        }
    }
    if let Some(items) = value.as_array() {
        if schema["maxItems"]
            .as_u64()
            .is_some_and(|max| items.len() > max as usize)
        {
            return false;
        }
        if let Some(item_schema) = schema.get("items") {
            if !items.iter().all(|item| validate_shape(item, item_schema)) {
                return false;
            }
        }
    }
    true
}

fn mutation_identity(conversation: &str, key: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(format!("{conversation}\0{key}")))
}

#[cfg(test)]
#[path = "tools_tests.rs"]
pub(crate) mod tests;
