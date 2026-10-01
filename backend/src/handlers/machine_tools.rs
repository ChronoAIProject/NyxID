//! Native machine MCP adapter. No token, credential, output, or path is audited.
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
        return tools::list_page(
            "machines",
            nodes
                .iter()
                .filter(|node| machines::granted(chat, node))
                .map(machines::metadata)
                .collect(),
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
    if !machines::granted(chat, &node) {
        return permission(state, chat, "machine", &node.id, &node.name).await;
    }
    let operation = tools::operation(name)
        .ok_or_else(|| AppError::NotFound("Machine tool not found".into()))?;
    machines::capable(&node, operation)?;
    if operation == Operation::Computer
        && !node.machine.as_ref().is_some_and(|profile| {
            arguments["tool"]
                .as_str()
                .is_some_and(|tool| profile.computer_tools.iter().any(|allowed| allowed == tool))
        })
    {
        return Err(AppError::MachineComputerUnavailable);
    }
    if name == "nyx__machine_request_control" {
        return super::machine_desktop::request_control(
            state,
            chat,
            &node,
            argument(&arguments, "reason")?,
        )
        .await;
    }
    crate::services::machine_desktop_service::agent_allowed(&state.db, &node.id).await?;
    if operation == Operation::Computer {
        crate::services::machine_desktop_service::open(
            &state.db,
            &chat.user_id,
            &node.id,
            Some(&chat.conversation_id),
        )
        .await?;
    }
    arguments["machine"] = json!(node.id);
    let mut login = None;
    if operation == Operation::FillLogin {
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
                    "message":"Saved-login typing is off on this single-user machine. Its commands run as the browser user and could read typed values. The owner can allow it in Nodes settings after reviewing the warning, or use the machine container or a separated VM."
                },
                "settings_path":"/nodes"
            }));
        }
        if !node.machine.as_ref().is_some_and(|p| p.saved_login_ready) {
            return Err(AppError::MachineBrowserUnavailable);
        }
        arguments["login"] = json!(row.id);
        login = Some(row);
    }
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
    if machines::confirmation(&node, operation, &arguments)
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
    let started = std::time::Instant::now();
    let result = match operation {
        Operation::SaveAttachment => save_attachment(state, chat, &node, &arguments).await,
        Operation::ShareFile => share_file(state, chat, &node, &arguments).await,
        _ => dispatch(state, &node, operation, arguments.clone()).await,
    };
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
            audit_service::log_async(
                state.db.clone(),
                Some(chat.user_id.clone()),
                "machine_operation".into(),
                Some(json!({
                    "node_id":node.id,
                    "operation":operation,
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
    if operation == Operation::Computer {
        attach_computer_images(state, chat, &mut result).await?;
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
    audit_service::log_async(
        state.db.clone(),
        Some(chat.user_id.clone()),
        "machine_operation".into(),
        Some(json!({
            "node_id":node.id,
            "operation":operation,
            "conversation_id":chat.conversation_id,
            "agent_role":chat.role,
            "services":declared_services.iter().map(|row|row.slug.as_str()).collect::<Vec<_>>(),
            "outcome":if result.get("error").is_some(){
                "refused"
            }else{
                "completed"
            },
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

async fn permission(
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
    let request = signed_request(state, node, operation, parameters).await?;
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
) -> AppResult<Request> {
    machines::capable(node, operation)?;
    let secret = node_service::signing_secret_from_node(&state.encryption_keys, node).await?;
    let mut request = Request {
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
) -> AppResult<()> {
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
    Ok(())
}

async fn save_attachment(
    state: &AppState,
    chat: &ChatAuthority,
    node: &Node,
    args: &Value,
) -> AppResult<Value> {
    let (_, bytes) = engine::read_attachment(
        &state.db,
        &state.encryption_keys,
        &chat.user_id,
        &chat.conversation_id,
        argument(args, "attachment_id")?,
    )
    .await?;
    if bytes.len() > crate::services::mcp_service::MAX_TOOL_IMAGE_BYTES {
        return Err(AppError::ValidationError(
            "Attachment exceeds the transfer limit".into(),
        ));
    }
    use sha2::{Digest, Sha256};
    let parameters = json!({
        "path":args["path"],
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
) -> AppResult<Value> {
    let bytes = transfer(
        state,
        node,
        Operation::ShareFile,
        json!({"path":args["path"]}),
        axum::body::Body::empty(),
        crate::services::mcp_service::MAX_TOOL_IMAGE_BYTES,
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

/// The attachment store encrypts one bounded image buffer. The node socket and
/// cross-replica hop carry bounded raw chunks, with no base64 body copies.
async fn transfer(
    state: &AppState,
    node: &Node,
    operation: Operation,
    mut parameters: Value,
    body: axum::body::Body,
    result_limit: usize,
) -> AppResult<Vec<u8>> {
    use crate::services::node_ws_manager::{ProxyResponseType, StreamChunk};
    parameters["max_bytes"] = json!(crate::services::mcp_service::MAX_TOOL_IMAGE_BYTES);
    let request = signed_request(state, node, operation, parameters).await?;
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
            _ => {
                return Err(AppError::ValidationError(
                    "Machine file transfer refused or interrupted".into(),
                ));
            }
        }
    }
}
