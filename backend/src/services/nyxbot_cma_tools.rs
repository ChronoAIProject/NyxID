//! CMA-native bridges. These never select/create an Agent or alter its lifecycle.
use super::{
    assistant_acknowledgement_service as acks, nyxbot_cma_agent_service as agents,
    nyxbot_cma_gateway::{CatalogCmaGateway, CmaGateway},
    nyxbot_cma_sandbox_service as sandboxes,
};
use crate::{
    errors::{AppError, AppResult},
    models::{assistant_agent::AssistantAgent, nyxbot_cma_agent::NyxbotCmaAgent},
};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde_json::{Value, json};

pub const NAMES: &[&str] = &[
    "list_cma_resources",
    "cma_save_attachment",
    "cma_request_owner",
];
pub const OWNER_TOOL: &str = "nyxid__cma_request_owner";
pub fn owns(name: &str) -> bool {
    NAMES.contains(&name)
}
fn refused() -> AppError {
    AppError::Forbidden("CMA requires this owner's enabled feature and current CMA grant".into())
}

pub async fn eligible(
    db: &Database,
    owner: &str,
    agent: Option<&AssistantAgent>,
) -> AppResult<bool> {
    if !feature_enabled(db, owner).await? {
        return Ok(false);
    }
    has_cma_grant(db, owner, agent).await
}

async fn feature_enabled(db: &Database, owner: &str) -> AppResult<bool> {
    super::feature_flag_service::personal_flag_enabled(
        db,
        owner,
        super::feature_flag_service::ASSISTANT_CMA_SANDBOXES_FLAG_KEY,
    )
    .await
}

async fn has_cma_grant(
    db: &Database,
    owner: &str,
    agent: Option<&AssistantAgent>,
) -> AppResult<bool> {
    let service =
        match super::catalog_service::get_downstream_service_by_slug(db, "cma", owner).await {
            Ok(service) => service,
            Err(AppError::NotFound(_)) => return Ok(false),
            Err(error) => return Err(error),
        };
    let Some(agent) = agent else {
        return Ok(true);
    };
    if agent.user_id != owner || agent.destroyed_at.is_some() {
        return Ok(false);
    }
    if agent.is_nyxbot() || agent.grants.platform_service_ids.contains(&service.id) {
        return Ok(true);
    }
    Ok(db.collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME).find_one(doc! {
        "_id":{"$in":&agent.grants.service_ids},"user_id":owner,"catalog_service_id":&service.id,"is_active":true,"deleted_at":null
    }).await?.is_some())
}
pub async fn chat_eligible(db: &Database, chat: &acks::ChatAuthority) -> AppResult<bool> {
    if !feature_enabled(db, &chat.user_id).await? {
        return Ok(false);
    }
    chat_has_cma_grant(db, chat).await
}

async fn chat_has_cma_grant(db: &Database, chat: &acks::ChatAuthority) -> AppResult<bool> {
    if chat.guest
        || chat.collaboration_thread
        || chat.org_agent_access.is_some()
        || super::assistant_service_execution::is_shared_chat(chat)
    {
        return Ok(false);
    }
    let agent = super::assistant_team_service::agent(db, &chat.user_id, &chat.agent_id).await?;
    has_cma_grant(db, &chat.user_id, Some(&agent)).await
}

pub fn schema(name: &str) -> Value {
    let id = json!({"type":"string","minLength":1,"maxLength":256});
    match name {
        "list_cma_resources" => {
            json!({"type":"object","properties":{"cursor":{"type":"string","maxLength":1024}},"required":[],"additionalProperties":false})
        }
        "cma_save_attachment" => {
            json!({"type":"object","properties":{"agent_id":id,"attachment_id":id,"path":{"type":"string","minLength":1,"maxLength":4096},"acknowledgement_id":id},"required":["agent_id","attachment_id","path"],"additionalProperties":false})
        }
        "cma_request_owner" => {
            json!({"type":"object","properties":{"agent_id":id,"reason":{"type":"string","minLength":1,"maxLength":500}},"required":["agent_id","reason"],"additionalProperties":false})
        }
        _ => json!({}),
    }
}
pub fn description(name: &str) -> String {
    match name {
        "list_cma_resources" => "Read personal CMA Sandbox allocation and live Agent facts with bounded pagination. This does not allocate, reserve or select an Agent; Agents can share an environment and context across chats.",
        "cma_save_attachment" => "Copy an original attachment from this conversation to a relative path in the selected CMA Agent. Returns path, size and SHA256; a different existing file is not overwritten. Requires the current CMA grant and action policy.",
        "cma_request_owner" => "Ask the owner to finish the stated step on the selected Agent's shared desktop. A card opens that exact Agent in NyxID. End this turn and wait; the owner's Done decision wakes a new event turn. Never click or infer completion yourself.",
        _ => "",
    }.into()
}
fn field<'a>(args: &'a Value, key: &str) -> AppResult<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::ValidationError("Invalid CMA tool arguments".into()))
}
async fn remote_id(db: &Database, owner: &str, input: &str) -> AppResult<String> {
    if input.starts_with("agt_") {
        super::nyxbot_cma_gateway::validate_remote_id(input)?;
        return Ok(input.into());
    }
    db.collection::<NyxbotCmaAgent>(crate::models::nyxbot_cma_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":input,"user_id":owner})
        .await?
        .map(|row| row.cma_agent_id)
        .ok_or_else(|| AppError::NotFound("CMA Agent not found".into()))
}
fn check_scope_inputs(
    scopes: &crate::models::agent_operation_scope::OperationScopes,
    connection: &str,
    service: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> AppResult<()> {
    let path = super::proxy_authorization::CanonicalPath::from_mcp_literal(path)?;
    // A producer-owned catalog route intersects all catalog-bound alias scopes,
    // including a grant whose former connection has since become unavailable.
    let catalog = (connection != service).then_some(service);
    super::agent_operation_scope_service::authorize(
        scopes, connection, catalog, None, method, &path, false, false,
    )?;
    super::agent_operation_scope_service::check_inputs(
        scopes,
        connection,
        catalog,
        None,
        method,
        &path,
        query,
        body,
        (!body.is_empty()).then_some("application/json"),
    )
}
pub(crate) async fn scope(
    db: &Database,
    chat: &acks::ChatAuthority,
    service: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    mode: crate::models::assistant_settings::AccessMode,
) -> AppResult<()> {
    let live =
        super::assistant_service_execution::live_authority(db, chat, "cma_native_dispatch").await?;
    if !chat_eligible(db, &live).await? {
        return Err(refused());
    }
    if !matches!(method, "GET" | "HEAD") && acks::effective_access_mode(db, &live).await? != mode {
        return Err(AppError::Conflict(
            "CMA action access mode changed; confirm again".into(),
        ));
    }
    key_scope(
        db,
        &live.user_id,
        &live.api_key_id,
        service,
        method,
        path,
        query,
        body,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn key_scope(
    db: &Database,
    owner: &str,
    key_id: &str,
    service: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> AppResult<()> {
    let key = super::key_service::get_api_key(db, owner, key_id).await?;
    if !crate::mw::auth::scope_allows_rest_proxy(&key.scopes) {
        return Err(refused());
    }
    if key
        .expires_at
        .is_some_and(|expiry| expiry <= chrono::Utc::now())
        || key
            .assistant_agent_owner_id
            .as_deref()
            .is_some_and(|bound| bound != owner)
    {
        return Err(refused());
    }
    let allowed = super::key_service::effective_allowed_service_ids(db, &key).await?;
    let connections: Vec<crate::models::user_service::UserService> = db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .find(
            doc! {"user_id":owner,"catalog_service_id":service,"is_active":true,"deleted_at":null},
        )
        .await?
        .try_collect()
        .await?;
    if !key.allow_all_services
        && !allowed.iter().any(|id| id == service)
        && !connections.iter().any(|row| allowed.contains(&row.id))
    {
        return Err(refused());
    }
    for connection in connections
        .iter()
        .filter(|row| key.allow_all_services || allowed.contains(&row.id))
    {
        // Applicable restrictions remain an intersection, even when an alias is used.
        check_scope_inputs(
            &key.assistant_operation_scopes,
            &connection.id,
            service,
            method,
            path,
            query,
            body,
        )?;
    }
    check_scope_inputs(
        &key.assistant_operation_scopes,
        service,
        service,
        method,
        path,
        query,
        body,
    )?;
    Ok(())
}

/// A task watch may observe through its current personal key without borrowing
/// a new live turn. Every additional read still obeys that key's exact scopes.
pub(crate) async fn watch_scope(
    db: &Database,
    watch: &crate::models::async_service_operation::AsyncServiceOperation,
    service: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> AppResult<()> {
    if method != "GET" || !body.is_empty() || watch.shared_execution.is_some() {
        return Err(refused());
    }
    let current=db.collection::<bson::Document>(crate::models::async_service_operation::COLLECTION_NAME).find_one(doc! {
        "_id":&watch.id,"user_id":&watch.user_id,"conversation_id":&watch.conversation_id,"api_key_id":&watch.api_key_id,
        "state":"waiting","lease_id":&watch.lease_id,"deadline":{"$gt":bson::DateTime::now()}
    }).await?;
    if current.is_none() {
        return Err(refused());
    }
    let mut chat = acks::for_key(db, &watch.user_id, Some(&watch.api_key_id))
        .await?
        .ok_or_else(refused)?;
    if chat.conversation_id != watch.conversation_id || chat.agent_id != watch.agent_id {
        return Err(refused());
    }
    // The admitted watch belongs to the original owner turn; a later guest
    // message cannot turn its narrowly scoped observation into guest authority.
    chat.guest = false;
    // Hiding the native/UI feature does not revoke already-admitted work.
    // Observation still needs the original personal scope and a current grant.
    if !chat_has_cma_grant(db, &chat).await? {
        return Err(refused());
    }
    key_scope(
        db,
        &watch.user_id,
        &watch.api_key_id,
        service,
        method,
        path,
        query,
        body,
    )
    .await
}

pub async fn execute(
    tools: &super::assistant_account_tools::AccountTools<'_>,
    chat: &acks::ChatAuthority,
    name: &str,
    args: &Value,
) -> AppResult<super::assistant_account_tools::ToolResult> {
    use super::assistant_account_tools::ToolResult;
    let live =
        super::assistant_service_execution::live_authority(tools.db, chat, "cma_native_tool")
            .await?;
    let chat = &live;
    if !chat_eligible(tools.db, chat).await? {
        return Err(refused());
    }
    let mode = acks::effective_access_mode(tools.db, chat).await?;
    // Select the producer-owned catalog identity, never a supplied connection.
    let catalog =
        super::catalog_service::get_downstream_service_by_slug(tools.db, "cma", &chat.user_id)
            .await?;
    let gateway =
        CatalogCmaGateway::from_parts(tools.db, tools.jwt_keys, tools.config, &catalog.id)
            .with_native_authority(chat, mode);
    gateway.verify_catalog(&chat.user_id, &catalog).await?;
    if name == "list_cma_resources" {
        // The gateway checks the live key and the exact inputs for every page,
        // detail read and effect, including requests made during registration.
        let sandbox_list = sandboxes::list_live(tools.db, &gateway, &chat.user_id, None).await?;
        let page = gateway
            .list_agents(&chat.user_id, args.get("cursor").and_then(Value::as_str))
            .await?;
        let mut rows = Vec::with_capacity(page.items.len());
        for remote in &page.items {
            rows.push(
                agents::register_live(tools.db, &gateway, &chat.user_id, &remote.id, None).await?,
            );
        }
        let value = json!({"sandboxes":sandbox_list,"agents":rows.iter().map(agent_summary).collect::<Vec<_>>(),"next_cursor":page.next_cursor});
        if value.to_string().len() > 64 * 1024 {
            return Err(AppError::Conflict("cma_resource_page_too_large".into()));
        }
        return Ok(ToolResult {
            value,
            is_error: false,
            permission_request: None,
        });
    }
    let remote = remote_id(tools.db, &chat.user_id, field(args, "agent_id")?).await?;
    let row = agents::register_live(tools.db, &gateway, &chat.user_id, &remote, None).await?;
    if name == "cma_request_owner" {
        let descriptor = gateway.desktop_descriptor(&chat.user_id, &remote).await?;
        if !descriptor.available {
            return Err(AppError::Conflict(
                descriptor
                    .reason
                    .unwrap_or_else(|| "desktop_unavailable".into()),
            ));
        }
        let bound = json!({"agent_id":remote,"agent_ref_id":row.id,"reason":field(args,"reason")?});
        let summary = format!(
            "Complete on the CMA shared desktop: {}",
            super::assistant_nyxagent::excerpt(field(args, "reason")?, 500)
        );
        let ack = acks::request(
            tools.db,
            chat,
            acks::Request {
                kind: "action",
                service: Some((&catalog.id, "cma", "CMA")),
                tool: Some(OWNER_TOOL),
                arguments: Some(&bound),
                summary: &summary,
                platform: true,
            },
        )
        .await?;
        return Ok(ToolResult {
            value: json!({"status":ack.status,"acknowledgement_id":ack.id,"desktop_url":ack.link,"instructions":"End this turn and wait. The owner completes the desktop step and clicks Done; NyxID then resumes this thread. Do not retry this request or infer completion."}),
            is_error: false,
            permission_request: None,
        });
    }
    let attachment = field(args, "attachment_id")?;
    let upload = super::assistant_upload_service::for_chat(tools.db, chat, attachment).await?;
    if upload.conversation_id != chat.conversation_id
        || upload.size < 0
        || upload.size > 32 * 1024 * 1024
    {
        return Err(AppError::ValidationError(
            "Use an attachment of at most 32 MiB from this conversation".into(),
        ));
    }
    let summary = format!(
        "Copy this conversation's attachment to CMA Agent {}",
        row.cma_agent_id
    );
    let tool = "nyxid__cma_save_attachment";
    let gate = acks::confirmation_needed(
        tools.db,
        chat,
        acks::ActionFacts::of(acks::classify_owner_action(tool, args)),
        false,
    )
    .await?;
    if let Some(need) = gate.ask() {
        if let Some(refusal) =
            acks::confirm_action(tools.db, chat, tool, args, &need, &summary).await?
        {
            return Ok(ToolResult {
                value: refusal,
                is_error: true,
                permission_request: None,
            });
        }
    }
    let (_, bytes) =
        super::assistant_upload_service::chat_bytes(tools.db, tools.keys, chat, attachment, false)
            .await?;
    let bytes = zeroize::Zeroizing::new(bytes);
    if !chat_eligible(tools.db, chat).await? {
        return Err(refused());
    }
    chat.ensure_live_turn(tools.db, None, None, "cma_save_attachment")
        .await?;
    let parent = row.cma_sandbox_id.as_deref().ok_or_else(refused)?;
    let sandbox = sandboxes::claim(tools.db, &gateway, &chat.user_id, parent, None).await?;
    if sandbox.retire_requested_at.is_some()
        || sandbox.admission_state
            == crate::models::nyxbot_cma_sandbox::SandboxAdmissionState::Retiring
    {
        return Err(AppError::Conflict("cma_sandbox_retiring".into()));
    }
    let saved = gateway
        .write_agent_file(&chat.user_id, &remote, field(args, "path")?, &bytes)
        .await?;
    if let Some(auto) = gate.auto_allowed() {
        acks::record_auto_allowed(tools.db, chat, tool, args, &auto, &summary).await;
    }
    Ok(ToolResult {
        value: json!({"path":saved.path,"size":saved.size_bytes,"sha256":saved.sha256}),
        is_error: false,
        permission_request: None,
    })
}
fn agent_summary(row: &NyxbotCmaAgent) -> Value {
    json!({"id":row.id,"agent_id":row.cma_agent_id,"sandbox_id":row.cma_sandbox_id,"workspace_id":row.cma_workspace_id,"status":row.resource_state,"last_observed":row.last_observed})
}
/// Bounded observations only. No transcript, prompt, profile or credentials.
pub async fn snapshot(
    db: &Database,
    owner: &str,
    agent: Option<&AssistantAgent>,
) -> AppResult<String> {
    if let Some(agent) = agent {
        let service =
            super::catalog_service::get_downstream_service_by_slug(db, "cma", owner).await?;
        for (id, scope) in &agent.operation_scopes {
            if id != &service.id && scope.catalog_service_id.as_deref() != Some(&service.id) {
                continue;
            }
            for path in ["/api/v1/sandboxes", "/api/v1/agents"] {
                if check_scope_inputs(
                    &agent.operation_scopes,
                    id,
                    &service.id,
                    "GET",
                    path,
                    None,
                    &[],
                )
                .is_err()
                {
                    return Ok("\nCMA resource snapshot is limited by this role's operation permissions. Query an explicitly allowed resource when needed.\n".into());
                }
            }
        }
    }
    let sandboxes = sandboxes::list(db, owner, None).await?;
    let agents: Vec<NyxbotCmaAgent> = db
        .collection(crate::models::nyxbot_cma_agent::COLLECTION_NAME)
        .find(doc! {"user_id":owner,"deleted_at":null})
        .sort(doc! {"last_observed":-1})
        .limit(8)
        .await?
        .try_collect()
        .await?;
    let resources = json!({"sandboxes":sandboxes.sandboxes.iter().map(|s|json!({"id":s.cma_sandbox_id,"status":s.observation_state,"creation":s.admission_state,"occupied":s.occupies_account_slot,"observed_at":s.last_observed})).take(8).collect::<Vec<_>>(),"agents":agents.iter().map(agent_summary).collect::<Vec<_>>()});
    Ok(format!(
        "\nCMA resources (last observed facts; query live status before acting; this is not a reservation):\n{resources}\n"
    ))
}

pub const GUIDANCE: &str = r#"
Use CMA when it helps the task. Decide whether to reuse an existing CMA Agent,
create a new Agent, or use other tools. Each account may have at most one
non-retired Sandbox, and that Sandbox may contain several Agents.
Create additional Agents inside that Sandbox with sandbox_id or workspace_id.
Create a Sandbox only when none exists, no creation is unresolved, and the task needs it.
Inspect resources as needed; a list is not a reservation.
Run work with cma__run_task using the selected agent_id, stream=true, and a stable
Idempotency-Key. If conversation is supplied, it must equal that agent_id.
Reusing an Agent shares its conversation and worktree, including across NyxBot chats.
Keep the exact body and Idempotency-Key when recovering an uncertain request.
Manage Agent lifecycle through tools. The user can inspect an Agent's events, history,
and available shared desktop on its execution page. The Sandboxes page manages the
parent environment and does not provide a desktop viewer.
Distinguish task cancellation, Agent stop/delete, and whole-Sandbox retirement.
Deleting an Agent does not retire its Sandbox or guarantee that resource charges stop.
Report observed queueing, failures and cleanup outcomes without claiming unobserved success.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::agent_operation_scope::{
        AgentOperationScope, OperationScopes, ScopedOperation,
    };
    use crate::models::downstream_service::ProxyOperationRule;

    #[test]
    fn empty_resource_query_is_valid_and_unknown_inputs_are_rejected() {
        super::super::assistant_account_tools::validate_arguments("list_cma_resources", &json!({}))
            .unwrap();
        assert!(
            super::super::assistant_account_tools::validate_arguments(
                "list_cma_resources",
                &json!({"agent_id":"agt_other"})
            )
            .is_err()
        );
    }

    #[test]
    fn native_attachment_checks_exact_agent_and_final_file_body() {
        let inputs = serde_json::from_value(json!({
            "path":{"agent_id":{"type":"exact","value":"agt_allowed"}},
            "body":{"type":"object","properties":{
                "path":{"type":"exact","value":"uploads/input.txt"},
                "content_base64":{"type":"exact","value":"aGk="},
                "directory":{"type":"exact","value":false}
            },"required":["path","content_base64","directory"]}
        }))
        .unwrap();
        let scopes: OperationScopes = [(
            "connection".into(),
            AgentOperationScope {
                revision: 1,
                catalog_service_id: Some("catalog".into()),
                operations: vec![ScopedOperation {
                    rule: ProxyOperationRule {
                        method: "POST".into(),
                        path_template: "/api/v1/agents/{agent_id}/files".into(),
                        ..Default::default()
                    },
                    inputs: Some(inputs),
                    ..Default::default()
                }],
            },
        )]
        .into();
        let body = br#"{"path":"uploads/input.txt","content_base64":"aGk=","directory":false}"#;
        check_scope_inputs(
            &scopes,
            "connection",
            "catalog",
            "POST",
            "/api/v1/agents/agt_allowed/files",
            None,
            body,
        )
        .unwrap();
        assert!(
            check_scope_inputs(
                &scopes,
                "connection",
                "catalog",
                "POST",
                "/api/v1/agents/agt_other/files",
                None,
                body
            )
            .is_err()
        );
        // A direct catalog grant cannot bypass the same alias-bound limits,
        // even when the connection no longer appears in the live lookup.
        assert!(
            check_scope_inputs(
                &scopes,
                "catalog",
                "catalog",
                "POST",
                "/api/v1/agents/agt_other/files",
                None,
                body
            )
            .is_err()
        );
        let changed = br#"{"path":"config/secret.txt","content_base64":"aGk=","directory":false}"#;
        assert!(
            check_scope_inputs(
                &scopes,
                "connection",
                "catalog",
                "POST",
                "/api/v1/agents/agt_allowed/files",
                None,
                changed
            )
            .is_err()
        );
        let changed =
            br#"{"path":"uploads/input.txt","content_base64":"b3RoZXI=","directory":false}"#;
        assert!(
            check_scope_inputs(
                &scopes,
                "connection",
                "catalog",
                "POST",
                "/api/v1/agents/agt_allowed/files",
                None,
                changed
            )
            .is_err()
        );
    }

    #[test]
    fn resource_pagination_checks_the_actual_cursor() {
        let inputs = serde_json::from_value(
            json!({"query":{"limit":{"rule":{"type":"exact","value":"8"}}}}),
        )
        .unwrap();
        let scopes: OperationScopes = [(
            "catalog".into(),
            AgentOperationScope {
                revision: 1,
                catalog_service_id: None,
                operations: vec![ScopedOperation {
                    rule: ProxyOperationRule {
                        method: "GET".into(),
                        path_template: "/api/v1/agents".into(),
                        ..Default::default()
                    },
                    inputs: Some(inputs),
                    ..Default::default()
                }],
            },
        )]
        .into();
        check_scope_inputs(
            &scopes,
            "catalog",
            "catalog",
            "GET",
            "/api/v1/agents",
            Some("limit=8"),
            &[],
        )
        .unwrap();
        assert!(
            check_scope_inputs(
                &scopes,
                "catalog",
                "catalog",
                "GET",
                "/api/v1/agents",
                Some("cursor=next&limit=8"),
                &[]
            )
            .is_err()
        );
    }
}
