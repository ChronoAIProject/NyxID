//! Native machine MCP adapter. No token, credential, output, or path is audited.
use crate::services::assistant_links::AssistantPage;
use crate::services::machine_access_service as access;
use crate::services::machine_activity_service as receipts;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::node::Node,
    services::{
        assistant_acknowledgement_service::{self as acks, ChatAuthority},
        assistant_nyxagent as engine, audit_service, machine_service as machines,
        machine_tools as tools, node_service,
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use nyxid_machine::authority::Authority;
use nyxid_machine::{Operation, Request};
use serde_json::{Value, json};

fn argument<'a>(value: &'a Value, key: &str) -> AppResult<&'a str> {
    value[key]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::ValidationError(format!("Missing {key}")))
}

pub async fn call(
    state: &AppState,
    chat: &ChatAuthority,
    name: &str,
    mut arguments: Value,
) -> AppResult<Value> {
    machines::caller(chat)?;
    if !tools::is_tool(name) {
        return Err(AppError::NotFound("Machine tool not found".into()));
    }
    if arguments.to_string().len() > 128 * 1024 {
        return Err(AppError::ValidationError(
            "Machine arguments exceed the size limit".into(),
        ));
    }
    if name == "nyx__saved_logins" {
        let rows =
            crate::services::saved_login_service::available(&state.db, &chat.user_id).await?;
        return tools::list_page("logins", rows.into_iter()
            .filter(|login| chat.is_orchestrator() || chat.saved_login_ids.contains(&login.id))
            .map(|row| json!({"id":row.id,"label":row.label,"allowed_origins":row.allowed_origins}))
            .collect(), &arguments, json!({}));
    }
    let nodes = machines::visible_nodes(&state.db, chat).await?;
    if name == "nyx__machine_list" {
        let services = crate::services::machine_gateway_service::services(
            &state.db,
            &chat.user_id,
            &chat.api_key_id,
        )
        .await?;
        let environment = services
            .iter()
            .map(|row| {
                let spec = crate::services::machine_gateway_service::environment(
                    std::slice::from_ref(row),
                )?;
                Ok(json!({
                    "service": row.slug,
                    "spec": spec,
                }))
            })
            .collect::<AppResult<Vec<_>>>()?;
        let instructions = if nodes.iter().any(|node| machines::granted(chat, node)) {
            tools::USE_INSTRUCTIONS
        } else {
            "No machine is available. Ask NyxBot for a machine setup link."
        };
        let visible = Box::pin(access::visible_assignments(&state.db, chat))
            .await?
            .into_iter()
            .map(|(node, assignment)| {
                let mut row = machines::metadata(&node);
                if assignment.mode == "separated" {
                    row["machine"]["roots"] = json!(["."]);
                    row["context_note"] = json!("Paths are relative to this agent context workspace; the secure and developer browsers are separate from other contexts. Full isolation requires a separate machine container or VM.");
                }
                row["access"] = json!(assignment);
                row
            })
            .collect();
        return tools::list_page(
            "machines",
            visible,
            &arguments,
            json!({
                "services": services.iter().map(|service| service.slug.as_str()).collect::<Vec<_>>(),
                "environment": environment,
                "instructions": instructions,
            }),
        );
    }

    let selector = argument(&arguments, "machine")?;
    let mut matches = nodes
        .into_iter()
        .filter(|node| node.id == selector || node.name == selector);
    let node = matches
        .next()
        .ok_or_else(|| AppError::NodeNotFound("Machine not found or not usable".into()))?;
    if matches.next().is_some() {
        return Err(AppError::ValidationError(
            "Several machines have this name; use the ID".into(),
        ));
    }
    let operation = tools::operation(name)
        .ok_or_else(|| AppError::NotFound("Machine tool not found".into()))?;
    let mut separated = false;
    if operation != Operation::JobCancel {
        match Box::pin(access::assignment(&state.db, chat, &node)).await {
            Ok(assignment) => separated = assignment.mode == "separated",
            Err(AppError::MachinePermissionRevoked) => {
                return permission(state, chat, "machine", &node.id, &node.name).await;
            }
            Err(error) => return Err(error),
        }
    }
    machines::capable(&node, operation)?;
    if operation == Operation::Browser
        && !node
            .machine
            .as_ref()
            .is_some_and(|profile| profile.browser_tools)
    {
        return Ok(json!({
            "error": {
                "code": 12416,
                "message": "This machine predates the managed browser tools. Offer nyxid__machine_update: it raises an owner card and guides the host step if needed. Do not ask the owner to diagnose browser settings."
            },
            "settings_path": AssistantPage::MachineSettings {node: &node.id}.path(),
        }));
    }
    if operation == Operation::Computer
        && !node.machine.as_ref().is_some_and(|profile| {
            arguments["tool"]
                .as_str()
                .is_some_and(|tool| profile.computer_tools.iter().any(|allowed| allowed == tool))
        })
    {
        return Ok(json!({
            "error": {
                "code": AppError::MachineComputerToolUnsupported.error_code(),
                "message": "computer_tool_not_supported: choose a tool from computer_tools below. For page content use nyx__machine_browser action=snapshot; owner screenshot attachments use browser=dev action=screenshot.",
                "computer_tools": node.machine.as_ref().map(|p| p.computer_tools.iter()
                    .take(64).map(|tool| tool.chars().take(128).collect::<String>()).collect::<Vec<_>>()).unwrap_or_default(),
            },
        }));
    }
    arguments["machine"] = json!(node.id);
    let mut login = None;
    if operation == Operation::FillLogin {
        if arguments.get("browser").is_some_and(|v| v != "secure") {
            return Err(AppError::ValidationError(
                "Saved logins are available only in the secure browser".into(),
            ));
        }
        let selector = argument(&arguments, "login")?;
        let rows =
            crate::services::saved_login_service::available(&state.db, &chat.user_id).await?;
        let mut found = rows
            .into_iter()
            .filter(|row| row.id == selector || row.label == selector);
        let row = found.next().ok_or_else(|| AppError::MachineLoginNotFound)?;
        if found.next().is_some() {
            return Err(AppError::ValidationError(
                "Several logins have this label; use the ID".into(),
            ));
        }
        if !chat.is_orchestrator() && !chat.saved_login_ids.contains(&row.id) {
            return permission(state, chat, "saved_login", &row.id, &row.label).await;
        }
        if !node.machine.as_ref().is_some_and(|p| p.browser_isolated)
            && !node.allow_single_user_saved_logins
        {
            return Ok(json!({
                "error":{
                    "code":12409,
                    "message":"Saved-login typing is off on this single-user machine. Its commands run as the browser user and could read typed values. The owner can allow it in Assistant → Machines settings after reviewing the warning, or use the machine container or a separated VM."
                },
                "settings_path": AssistantPage::Machines.path()
            }));
        }
        if !separated && !node.machine.as_ref().is_some_and(|p| p.saved_login_ready) {
            return Err(AppError::MachineBrowserUnavailable);
        }
        arguments["login"] = json!(row.id);
        login = Some(row);
    }
    Box::pin(access::authorize(
        &state.db, chat, &node, operation, &arguments,
    ))
    .await?;
    let declared_services = if operation == Operation::Exec {
        let requested: Vec<String> = serde_json::from_value(
            arguments
                .get("services")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )
        .map_err(|_| {
            AppError::ValidationError("services must be a list of service slugs or IDs".into())
        })?;
        let available = if requested.is_empty() {
            Vec::new()
        } else {
            crate::services::machine_gateway_service::services(
                &state.db,
                &chat.user_id,
                &chat.api_key_id,
            )
            .await?
        };
        let selected = crate::services::machine_gateway_service::declare(&requested, available)?;
        arguments["services"] = json!(selected.iter().map(|row| &row.slug).collect::<Vec<_>>());
        selected
    } else {
        Vec::new()
    };
    arguments["machine_access_revision"] = json!(
        Box::pin(access::policy(&state.db, &chat.agent_id))
            .await?
            .revision
    );
    // Arbitrary commands, writes and mutating desktop input can destroy data.
    // Filling a checked login field and asking the owner for control change
    // state, but do not themselves remove data or grant arbitrary execution.
    let read_only = !machines::changing(operation, &arguments);
    let destructive = matches!(
        operation,
        Operation::Exec | Operation::WriteFile | Operation::SaveAttachment | Operation::JobCancel
    ) || (matches!(operation, Operation::Computer | Operation::Browser)
        && !read_only);
    let webhook_confirmation = acks::webhook_confirmation_required(chat, read_only, destructive);
    if webhook_confirmation {
        if let Some(refusal) =
            acks::webhook_action_gate(&state.db, chat, name, &arguments, read_only, destructive)
                .await?
        {
            return Ok(refusal);
        }
        // One digest-bound owner decision satisfies both policies. Never
        // consume it twice when machine_confirm also requires confirmation.
    } else if machines::confirmation(&node, operation, &arguments)
        || login.as_ref().is_some_and(|row| row.confirm_each_sign_in)
    {
        let approved = if let Some(id) = arguments["acknowledgement_id"].as_str() {
            acks::consume_action(&state.db, chat, id, name, &arguments).await?
        } else {
            false
        };
        if !approved {
            let row = acks::request(
                &state.db,
                chat,
                acks::Request {
                    kind: "action",
                    service: None,
                    tool: Some(name),
                    arguments: Some(&arguments),
                    summary: &format!(
                        "Allow {name} on {}{}",
                        node.name,
                        if operation == Operation::Exec {
                            format!(
                                "; declared services: {}",
                                declared_services
                                    .iter()
                                    .map(|row| row.slug.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        } else {
                            String::new()
                        }
                    ),
                    platform: false,
                },
            )
            .await?;
            return Ok(acks::refusal(&row));
        }
    }
    // Fence against Stop/settlement before issuing any signed machine work.
    let turn_id = chat
        .turn_id
        .as_deref()
        .filter(|_| !chat.turn_stopped)
        .ok_or(AppError::MachineTurnStopped)?;
    let admitted = state.db.collection::<crate::models::assistant_conversation::AssistantConversation>(
        crate::models::assistant_conversation::COLLECTION_NAME,
    ).update_one(
        mongodb::bson::doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,"active_turn.turn_id":turn_id,"active_turn.stop_requested":false},
        mongodb::bson::doc! {"$addToSet":{"active_turn.machine_node_ids":&node.id}},
    ).await?;
    if admitted.matched_count != 1 {
        return Err(AppError::MachineTurnStopped);
    }
    arguments["conversation_id"] = json!(chat.conversation_id);
    arguments["turn_id"] = json!(turn_id);
    // Desktop state is opened only after authority admission. A separated
    // authority carries the signed context id that the node must use; opening
    // it before admission would create a legacy desktop row and route control
    // to the wrong browser profile.
    if matches!(operation, Operation::Job | Operation::JobCancel) {
        machines::job(&state.db, chat, &node.id, argument(&arguments, "job_id")?).await?;
    }
    let job = if operation == Operation::Exec {
        arguments["environment"] = json!(crate::services::machine_gateway_service::environment(
            &declared_services
        )?);
        let job = machines::issue_job(
            &state.db,
            chat,
            &node,
            arguments["timeout_secs"].as_u64().unwrap_or(120),
            crate::services::machine_gateway_service::declared(&declared_services),
        )
        .await?;
        arguments["job_id"] = json!(job.id);
        arguments["runtime_id"] = json!(job.runtime_id);
        arguments["conversation_id"] = json!(chat.conversation_id);
        Some(job)
    } else {
        None
    };
    let authority = match Box::pin(access::admit(
        &state.db,
        chat,
        &node,
        operation,
        &arguments,
        job.as_ref().map(|job| job.id.as_str()),
    ))
    .await
    {
        Ok(authority) => authority,
        Err(error) => {
            if let Some(job) = &job {
                machines::finish(&state.db, &job.id).await?;
            }
            return Err(error);
        }
    };
    let context_id = authority
        .as_deref()
        .filter(|authority| authority.mode == "separated")
        .map(|authority| authority.context_id.as_str());
    let display = if name == "nyx__machine_request_control" {
        nyxid_machine::desktop::Display::from_parameters(&arguments)
            .map_err(|message| AppError::ValidationError(message.into()))?
    } else if operation == Operation::Browser && arguments["browser"] == "dev" {
        nyxid_machine::desktop::Display::Dev
    } else {
        nyxid_machine::desktop::Display::Secure
    };
    if name != "nyx__machine_request_control" {
        let allowed = if node
            .machine
            .as_ref()
            .is_some_and(|profile| profile.os == "linux")
        {
            if matches!(
                operation,
                Operation::Browser | Operation::Computer | Operation::FillLogin
            ) {
                crate::services::machine_desktop_service::agent_display_allowed_for_context(
                    &state.db, &node.id, display, context_id,
                )
                .await
            } else {
                crate::services::machine_desktop_service::agent_allowed_for_context(
                    &state.db, &node.id, context_id,
                )
                .await
            }
        } else {
            // Native macOS browsers share a physical desktop.
            crate::services::machine_desktop_service::agent_allowed(&state.db, &node.id).await
        };
        if let Err(error) = allowed {
            if let Some(authority) = &authority {
                let _ = state
                    .db
                    .collection::<mongodb::bson::Document>(crate::models::machine_access::LEASES)
                    .delete_one(mongodb::bson::doc! {"_id": &authority.lease_id})
                    .await;
            }
            return Err(error);
        }
    }
    if name == "nyx__machine_request_control" {
        let result = super::machine_desktop::request_control(
            state,
            chat,
            &node,
            argument(&arguments, "reason")?,
            display,
            context_id,
        )
        .await;
        if let Some(authority) = &authority {
            let _ = state
                .db
                .collection::<mongodb::bson::Document>(crate::models::machine_access::LEASES)
                .delete_one(mongodb::bson::doc! {"_id": &authority.lease_id})
                .await;
        }
        return result;
    }
    if matches!(operation, Operation::Computer | Operation::Browser) {
        crate::services::machine_desktop_service::open_display_for_context(
            &state.db,
            &chat.user_id,
            &node.id,
            Some(&chat.conversation_id),
            context_id,
            display,
        )
        .await?;
    }
    if let (Some(job), Some(authority)) = (&job, &authority) {
        state.db.collection::<mongodb::bson::Document>(crate::models::machine_job::COLLECTION_NAME)
            .update_one(mongodb::bson::doc!{"_id":&job.id},mongodb::bson::doc!{"$set":{"machine_authority":mongodb::bson::to_bson(authority).map_err(|_|AppError::MachineAuthorityStale)?}}).await?;
    }
    if let Some(login) = &login {
        let field = argument(&arguments, "field")?.to_owned();
        let value = crate::services::saved_login_service::materialize(
            &state.encryption_keys,
            login,
            &field,
            Utc::now().timestamp().max(0) as u64,
        )
        .await?;
        arguments["value"] = json!(value.as_str());
        arguments["allowed_origins"] = json!(login.allowed_origins);
    }
    let mut receipt = receipts::receipt(chat, &node.id, operation, &arguments);
    receipt.context_mode = Some(
        if separated {
            "separated"
        } else {
            "shared_legacy"
        }
        .into(),
    );
    receipt.preview_enabled = receipts::preview_enabled(&state.db, chat).await?;
    receipts::record(&state.db, chat, &receipt).await?;
    let started = std::time::Instant::now();
    let mut result = match operation {
        Operation::SaveAttachment => {
            save_attachment(state, chat, &node, &arguments, authority.clone()).await
        }
        Operation::ShareFile => share_file(state, chat, &node, &arguments, authority.clone()).await,
        _ => {
            dispatch_authorized(
                state,
                &node,
                operation,
                arguments.clone(),
                authority.clone(),
            )
            .await
        }
    };
    if let Some(authority) = &authority
        && operation != Operation::JobCancel
    {
        let current = state.db.collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .find_one(mongodb::bson::doc! {"_id":&authority.agent_id,"destroyed_at":mongodb::bson::Bson::Null,format!("machine_access.assignments.{}.revision",node.id):authority.revision})
            .projection(mongodb::bson::doc! {"_id":1}).await?;
        if current.is_none() {
            result = Err(AppError::MachinePermissionRevoked);
        }
    }
    if let Some(authority) = &authority
        && (job.is_none()
            || result.as_ref().is_ok_and(|r| r["status"] == "finished")
            || result.is_err())
    {
        let _ = state
            .db
            .collection::<mongodb::bson::Document>(crate::models::machine_access::LEASES)
            .delete_one(mongodb::bson::doc! {"_id":&authority.lease_id})
            .await;
    }
    if let Some(job) = job
        && (result.is_err()
            || result
                .as_ref()
                .is_ok_and(|r| r["status"] == "finished" || r.get("error").is_some()))
    {
        machines::finish(&state.db, &job.id).await?;
    }
    if matches!(operation, Operation::Job | Operation::JobCancel)
        && result.as_ref().is_ok_and(|r| r["status"] == "finished")
    {
        machines::finish(&state.db, argument(&arguments, "job_id")?).await?;
    }
    // This copy is only needed for dispatch; cards and audit never contain it.
    if let Some(Value::String(value)) = arguments.get_mut("value") {
        use zeroize::Zeroize;
        value.zeroize();
    }
    let mut result = match result {
        Ok(result) => result,
        Err(error) => {
            receipt.status = if error.error_code() == 12418 {
                "cancelled"
            } else {
                "error"
            }
            .into();
            receipt.error_code = Some(error.error_code());
            receipt.duration_ms = Some(started.elapsed().as_millis() as u64);
            let _ = receipts::record(&state.db, chat, &receipt).await;
            audit_service::log_async(
                state.db.clone(),
                Some(chat.user_id.clone()),
                "machine_operation".into(),
                Some(json!({
                    "node_id":node.id,
                    "operation":operation,
                    "operation_id":receipt.operation_id,
                    "activity_id":receipts::activity_id(),
                    "agent_id":chat.agent_id,
                    "action":receipt.action,
                    "job_id":receipt.job_id,
                    "conversation_id":chat.conversation_id,
                    "agent_role":chat.role,
                    "services":declared_services.iter().map(|row|row.slug.as_str()).collect::<Vec<_>>(),
                    "outcome":"failed",
                    "code":error.error_code(),
                    "duration_ms":started.elapsed().as_millis() as u64,
                    "card_used":arguments.get("acknowledgement_id").is_some()
                })),
                None,
                None,
                Some(chat.api_key_id.clone()),
                None,
            );
            return Err(error);
        }
    };
    if matches!(operation, Operation::Computer | Operation::Browser) {
        receipt.screenshot_id = attach_computer_images(state, chat, &mut result).await?;
    }
    if let Some(login) = login {
        if result["status"] == "filled" {
            crate::services::saved_login_service::record_use(&state.db, &login.id).await?;
            result = json!({
                "filled":arguments["field"],
                "login":login.label,
                "origin":result["origin"]
            });
        } else if result["status"] == "refused" {
            let error = match result["reason"].as_str() {
                Some("origin_mismatch") => AppError::MachineLoginOriginMismatch,
                Some("wrong_field" | "focus_changed" | "no_suitable_focused_field") => {
                    AppError::MachineLoginWrongField
                }
                _ => AppError::MachineBrowserUnavailable,
            };
            result = crate::services::assistant_account_tools::error_result(error).value;
        }
        audit_service::log_async(
            state.db.clone(),
            Some(chat.user_id.clone()),
            "machine_login_filled".into(),
            Some(json!({
                "login_id":login.id,
                "node_id":node.id,
                "field":arguments["field"],
                "origin":result["origin"],
                "outcome":if result.get("error").is_some(){
                    "refused"
                }else{
                    "filled"
                }
            })),
            None,
            None,
            Some(chat.api_key_id.clone()),
            None,
        );
    }
    receipt.status = receipts::outcome(&result, operation).into();
    receipt.exit_code = result["exit_code"].as_i64();
    receipt.bytes = receipts::transferred_bytes(operation, &arguments, &result);
    receipt.duration_ms = Some(started.elapsed().as_millis() as u64);
    receipt.error_code = result["error"]["code"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok());
    if let (Some(job_id), Some(exit_code)) = (&receipt.job_id, receipt.exit_code) {
        // Additional outcome metadata; do not resurrect a cancelled job.
        let _ = state.db.collection::<mongodb::bson::Document>(crate::models::machine_job::COLLECTION_NAME)
            .update_one(mongodb::bson::doc!{"_id":job_id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"state":{"$ne":"cancelled"}},
                mongodb::bson::doc!{"$set":{"exit_code":exit_code}}).await;
    }
    if operation == Operation::JobCancel
        && receipt.status == "cancelled"
        && let Some(job_id) = &receipt.job_id
    {
        // Presentation only: the existing job lifecycle remains authoritative.
        let _ = state.db.collection::<mongodb::bson::Document>(crate::models::machine_job::COLLECTION_NAME)
            .update_one(mongodb::bson::doc!{"_id":job_id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id},
                mongodb::bson::doc!{"$set":{"receipt_cancelled":true}}).await;
    }
    if receipts::record(&state.db, chat, &receipt).await.is_err()
        || receipts::store_preview(state, chat, &mut receipt, &result)
            .await
            .is_err()
    {
        tracing::warn!("Machine receipt preview could not be retained");
    }
    audit_service::log_async(
        state.db.clone(),
        Some(chat.user_id.clone()),
        "machine_operation".into(),
        Some(json!({
            "node_id":node.id,
            "operation":operation,
            "operation_id":receipt.operation_id,
            "activity_id":receipts::activity_id(),
            "agent_id":chat.agent_id,
            "action":receipt.action,
            "job_id":receipt.job_id,
            "conversation_id":chat.conversation_id,
            "agent_role":chat.role,
            "services":declared_services.iter().map(|row|row.slug.as_str()).collect::<Vec<_>>(),
            "outcome":receipt.status,
            "exit_code":result["exit_code"].as_i64(),
            "duration_ms":started.elapsed().as_millis() as u64,
            "bytes":result.to_string().len(),
            "card_used":arguments.get("acknowledgement_id").is_some()
        })),
        None,
        None,
        Some(chat.api_key_id.clone()),
        None,
    );
    Ok(tools::bounded_result(result))
}

pub(crate) async fn permission(
    state: &AppState,
    chat: &ChatAuthority,
    kind: &str,
    id: &str,
    label: &str,
) -> AppResult<Value> {
    let (row, created) = acks::request_tracked(
        &state.db,
        chat,
        acks::Request {
            kind,
            service: Some((id, id, label)),
            tool: None,
            arguments: None,
            summary: &format!("Use {kind} {label}"),
            platform: false,
        },
    )
    .await?;
    if created {
        super::assistant_team::permission_requested(state, chat, &row).await;
    }
    Ok(acks::refusal(&row))
}

pub async fn dispatch(
    state: &AppState,
    node: &Node,
    operation: Operation,
    parameters: Value,
) -> AppResult<Value> {
    dispatch_authorized(state, node, operation, parameters, None).await
}
async fn dispatch_authorized(
    state: &AppState,
    node: &Node,
    operation: Operation,
    parameters: Value,
    authority: Option<Box<Authority>>,
) -> AppResult<Value> {
    let request = signed_request(state, node, operation, parameters, authority).await?;
    Ok(state
        .node_dispatch
        .machine_request_with_node(request, node)
        .await?
        .result)
}

async fn signed_request(
    state: &AppState,
    node: &Node,
    operation: Operation,
    parameters: Value,
    authority: Option<Box<Authority>>,
) -> AppResult<Request> {
    machines::capable(node, operation)?;
    let secret = node_service::signing_secret_from_node(&state.encryption_keys, node).await?;
    let mut request = Request {
        version: if authority.is_some() { 2 } else { 1 },
        authority,
        request_id: uuid::Uuid::new_v4().to_string(),
        node_id: node.id.clone(),
        operation,
        parameters,
        timestamp: Utc::now().timestamp(),
        nonce: uuid::Uuid::new_v4().to_string(),
        signature: String::new(),
    };
    request.signature = nyxid_machine::signing::sign(&request, &secret);
    Ok(request)
}

async fn attach_computer_images(
    state: &AppState,
    chat: &ChatAuthority,
    result: &mut Value,
) -> AppResult<Option<String>> {
    let mut screenshot_id = None;
    if let Some(content) = result["content"].as_array_mut() {
        for item in content {
            if item["type"] != "image" {
                continue;
            }
            let encoded = item["data"].as_str().unwrap_or_default();
            if encoded.len() > 7 * 1024 * 1024 {
                return Err(AppError::ValidationError(
                    "Machine image exceeds the limit".into(),
                ));
            }
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| AppError::ValidationError("Invalid machine image".into()))?;
            let media =
                crate::services::mcp_service::tool_media(200, item["mimeType"].as_str(), &bytes)
                    .ok_or_else(|| {
                        AppError::ValidationError("Invalid machine image type or size".into())
                    })?;
            let attached = engine::attach_image(
                &state.db,
                &state.encryption_keys,
                &chat.user_id,
                &chat.conversation_id,
                "Machine screenshot",
                &media.content_type,
                &media.bytes,
            )
            .await?;
            screenshot_id = attached.as_ref().map(|meta| meta.id.clone());
            *item = json!({
                "type":"text",
                "text":if attached.is_some(){
                    "Image displayed to the owner in this conversation. Pixels are not in the model context."
                }else{
                    "The image could not be attached: no live turn or attachment limit reached."
                }
            });
        }
    }
    Ok(screenshot_id)
}

async fn save_attachment(
    state: &AppState,
    chat: &ChatAuthority,
    node: &Node,
    args: &Value,
    authority: Option<Box<Authority>>,
) -> AppResult<Value> {
    let id = argument(args, "attachment_id")?;
    let is_upload = state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .find_one(mongodb::bson::doc! {"_id": id, "origin": "user_upload"})
        .projection(mongodb::bson::doc! {"_id":1})
        .await?
        .is_some();
    let (_, bytes) = if is_upload {
        crate::services::assistant_upload_service::chat_bytes(
            &state.db,
            &state.encryption_keys,
            chat,
            id,
            false,
        )
        .await?
    } else {
        engine::read_attachment(
            &state.db,
            &state.encryption_keys,
            &chat.user_id,
            &chat.conversation_id,
            argument(args, "attachment_id")?,
        )
        .await?
    };
    if bytes.len() > crate::services::attachment_extraction::MAX_BYTES {
        return Err(AppError::ValidationError(
            "Attachment exceeds the transfer limit".into(),
        ));
    }
    use sha2::{Digest, Sha256};
    let parameters = json!({
        "path":args["path"],
        "conversation_id":args["conversation_id"],
        "turn_id":args["turn_id"],
        "size":bytes.len(),
        "sha256":hex::encode(Sha256::digest(&bytes))
    });
    let result = transfer(
        state,
        node,
        Operation::SaveAttachment,
        parameters,
        axum::body::Body::from(bytes),
        4096,
        authority,
    )
    .await?;
    serde_json::from_slice(&result)
        .map_err(|_| AppError::ValidationError("Invalid file transfer response".into()))
}

async fn share_file(
    state: &AppState,
    chat: &ChatAuthority,
    node: &Node,
    args: &Value,
    authority: Option<Box<Authority>>,
) -> AppResult<Value> {
    let bytes = transfer(
        state,
        node,
        Operation::ShareFile,
        json!({"path":args["path"],"conversation_id":args["conversation_id"],"turn_id":args["turn_id"]}),
        axum::body::Body::empty(),
        crate::services::mcp_service::MAX_TOOL_IMAGE_BYTES,
        authority,
    )
    .await?;
    let kind = ["image/png", "image/jpeg", "image/gif", "image/webp"]
        .into_iter()
        .find(|kind| crate::services::mcp_service::image_magic_matches(kind, &bytes))
        .ok_or_else(|| {
            AppError::ValidationError("Only PNG, JPEG, GIF and WebP images may be shared".into())
        })?;
    let attached = engine::attach_image(
        &state.db,
        &state.encryption_keys,
        &chat.user_id,
        &chat.conversation_id,
        "Machine image",
        kind,
        &bytes,
    )
    .await?;
    Ok(json!({
        "attached":attached.is_some(),
        "bytes":bytes.len(),
        "message":"The image is shown only to the owner in this conversation."
    }))
}

fn attachment_transfer_limit(operation: Operation, parameters: &Value) -> AppResult<u64> {
    match operation {
        // Sign the actual file size, so existing <=5 MiB saves still work on
        // older nodes. Updated nodes accept user documents/images up to 20 MiB.
        Operation::SaveAttachment => parameters["size"]
            .as_u64()
            .filter(|size| {
                *size > 0 && *size <= crate::services::attachment_extraction::MAX_BYTES as u64
            })
            .ok_or(AppError::MachineLimitExceeded),
        Operation::ShareFile => Ok(crate::services::mcp_service::MAX_TOOL_IMAGE_BYTES as u64),
        _ => Err(AppError::MachineLimitExceeded),
    }
}

/// The attachment store encrypts bounded buffers. The node socket and
/// cross-replica hop carry bounded raw chunks, with no base64 body copies.
async fn transfer(
    state: &AppState,
    node: &Node,
    operation: Operation,
    mut parameters: Value,
    body: axum::body::Body,
    result_limit: usize,
    authority: Option<Box<Authority>>,
) -> AppResult<Vec<u8>> {
    use crate::services::node_ws_manager::{ProxyResponseType, StreamChunk};
    parameters["max_bytes"] = json!(attachment_transfer_limit(operation, &parameters)?);
    let request = signed_request(state, node, operation, parameters, authority).await?;
    let response = state
        .node_dispatch
        .proxy_upload(request, body)
        .await
        .map_err(|error| error.error)?;
    let ProxyResponseType::Streaming(mut stream) = response else {
        return Err(AppError::ValidationError(
            "Machine did not open a file stream".into(),
        ));
    };
    let mut bytes = Vec::new();
    let mut started = false;
    loop {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(60), stream.recv())
            .await
            .map_err(|_| AppError::NodeProxyTimeout)?
            .ok_or_else(|| AppError::NodeOffline("File transfer interrupted".into()))?;
        match chunk {
            StreamChunk::Start { status, .. } if !started && status == 200 => started = true,
            StreamChunk::Data(data) if started => {
                if bytes.len().saturating_add(data.len()) > result_limit {
                    return Err(AppError::MachineLimitExceeded);
                }
                bytes.extend_from_slice(&data);
            }
            StreamChunk::End if started => return Ok(bytes),
            // The node emits fixed protocol reason codes, never child stderr.
            StreamChunk::Error(reason) if reason == "machine_authority_stale" => {
                return Err(AppError::MachineAuthorityStale);
            }
            StreamChunk::Error(reason) if reason == "machine_turn_stopped" => {
                return Err(AppError::MachineTurnStopped);
            }
            StreamChunk::Error(reason) if reason == "owner_in_control" => {
                return Err(AppError::MachineOwnerInControl);
            }
            _ => {
                return Err(AppError::ValidationError(
                    "Machine file transfer refused or interrupted".into(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod attachment_transfer_tests {
    use super::*;

    #[test]
    fn assistant_attachment_transfers_preserve_legacy_images_and_bound_large_uploads() {
        let upload = crate::services::attachment_extraction::MAX_BYTES as u64;
        assert_eq!(
            attachment_transfer_limit(Operation::SaveAttachment, &json!({"size": 512})).unwrap(),
            512
        );
        assert_eq!(
            attachment_transfer_limit(Operation::SaveAttachment, &json!({"size": upload})).unwrap(),
            upload
        );
        assert!(
            attachment_transfer_limit(Operation::SaveAttachment, &json!({"size": upload + 1}))
                .is_err()
        );
        assert!(attachment_transfer_limit(Operation::SaveAttachment, &json!({})).is_err());
        assert_eq!(
            attachment_transfer_limit(Operation::ShareFile, &json!({"size": upload})).unwrap(),
            5 * 1024 * 1024
        );
    }
}
