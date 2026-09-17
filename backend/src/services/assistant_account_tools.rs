//! Closed native MCP inventory. No HTTP loopback and no secret-bearing DTOs.
use crate::{
    config::AppConfig,
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{api_key::ApiKey, channel_bot::ChannelBot, channel_conversation::ChannelConversation},
    mw::auth::{AuthMethod, AuthUser},
    services::{
        agent_binding_service, approval_service, assistant_acknowledgement_service as acks,
        assistant_agent_credential_service::ASSISTANT_PLATFORM,
        audit_service, channel_adapters, channel_bot_service, channel_routing_service, key_service,
        mcp_service::{McpToolEndpoint, McpToolService, McpToolSource},
        node_service,
        node_ws_manager::NodeWsManager,
        platform_key_service,
        provider_token_exchange_service::TokenExchangeCache,
        unified_key_service, user_service_service,
    },
};
use mongodb::{Database, bson::doc};
use serde_json::{Value, json};
use std::sync::Arc;

pub const TOOL_NAMES: &[&str] = &[
    "list_agent_keys",
    "get_agent_key",
    "update_agent_key",
    "delete_agent_key",
    "list_agent_key_bindings",
    "bind_agent_key_credential",
    "unbind_agent_key_credential",
    "list_channel_bots",
    "get_channel_bot",
    "update_channel_bot",
    "delete_channel_bot",
    "list_channel_routes",
    "set_channel_route",
    "delete_channel_route",
    "list_my_services",
    "set_service_enabled",
    "delete_service",
    "list_nodes",
    "delete_node",
    "list_approval_configs",
    "set_approval_mode",
    "list_pending_approvals",
];
const MAX_ITEMS: usize = 100;
const MAX_RESULT_BYTES: usize = 64 * 1024;

pub fn destructive(name: &str) -> bool {
    matches!(
        name,
        "delete_agent_key"
            | "unbind_agent_key_credential"
            | "delete_channel_bot"
            | "delete_channel_route"
            | "delete_service"
            | "delete_node"
    )
}

pub fn schema(name: &str) -> Value {
    let string = json!({"type": "string", "minLength": 1, "maxLength": 200});
    let boolean = json!({"type": "boolean"});
    let mut props = serde_json::Map::new();
    let mut required: Vec<&str> = Vec::new();
    let mut add = |key: &'static str, value: Value, mandatory: bool| {
        props.insert(key.into(), value);
        if mandatory {
            required.push(key);
        }
    };
    match name {
        "get_agent_key"
        | "update_agent_key"
        | "delete_agent_key"
        | "list_agent_key_bindings"
        | "bind_agent_key_credential"
        | "unbind_agent_key_credential" => add("api_key_id", string.clone(), true),
        "get_channel_bot" | "update_channel_bot" | "delete_channel_bot" => {
            add("bot_id", string.clone(), true)
        }
        "delete_channel_route" => add("route_id", string.clone(), true),
        "set_service_enabled" | "delete_service" | "set_approval_mode" => {
            add("service_id", string.clone(), true)
        }
        "delete_node" => add("node_id", string.clone(), true),
        _ => {}
    }
    match name {
        "update_agent_key" => {
            add("name", string.clone(), false);
            add(
                "description",
                json!({"type": "string", "maxLength": 2000}),
                false,
            );
            add(
                "platform",
                json!({"type": ["string", "null"], "maxLength": 64}),
                false,
            );
            add(
                "callback_url",
                json!({"type": ["string", "null"], "maxLength": 2048}),
                false,
            );
            for field in ["rate_limit_per_second", "rate_limit_burst"] {
                add(
                    field,
                    json!({"type": ["integer", "null"], "minimum": 1, "maximum": 1000000}),
                    false,
                );
            }
            for field in [
                "allow_all_services",
                "allow_auto_connected_services",
                "allow_all_nodes",
            ] {
                add(field, boolean.clone(), false);
            }
            for field in ["allowed_service_slugs", "allowed_node_ids"] {
                add(
                    field,
                    json!({"type": "array", "maxItems": 100, "items": string}),
                    false,
                );
            }
        }
        "bind_agent_key_credential" => {
            add("user_service_id", string.clone(), true);
            add("user_api_key_id", string.clone(), true);
        }
        "unbind_agent_key_credential" => add("binding_id", string.clone(), true),
        "update_channel_bot" => {
            add("label", string.clone(), false);
            add("app_id", string.clone(), false);
        }
        "list_channel_routes" => add("bot_id", string.clone(), false),
        "set_channel_route" => {
            add("route_id", string.clone(), false);
            add("bot_id", string.clone(), false);
            add("agent_api_key_id", string.clone(), true);
            add("platform_conversation_id", string.clone(), false);
            add(
                "platform_conversation_type",
                json!({"type": "string", "enum": ["private", "group", "channel"]}),
                false,
            );
            add("default_agent", boolean.clone(), false);
            add("allow_agent_initiated", boolean.clone(), false);
        }
        "set_service_enabled" => add("enabled", boolean, true),
        "set_approval_mode" => {
            add("approval_required", boolean, false);
            add(
                "approval_mode",
                json!({"type": "string", "enum": ["per_request", "grant"]}),
                true,
            );
        }
        _ => {}
    }
    if destructive(name) {
        add("acknowledgement_id", string, false);
    }
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

fn description(name: &str) -> String {
    let purpose = match name {
        "list_agent_keys" => "List agent keys without secrets.",
        "get_agent_key" => "Inspect an agent key's permissions and settings.",
        "update_agent_key" => {
            "Update agent key permissions, bindings scope and non-secret \
                settings. Assistant chat keys cannot be modified."
        }
        "delete_agent_key" => "Delete an agent key and revoke its credentials.",
        "list_agent_key_bindings" => "List an agent key's credential bindings without credentials.",
        "bind_agent_key_credential" => {
            "Bind an existing credential to an agent key and connected \
                service. Never enter a raw credential."
        }
        "unbind_agent_key_credential" => "Delete an agent credential binding.",
        "list_channel_bots" => "List channel bots without credentials or tokens.",
        "get_channel_bot" => "Inspect a channel bot's non-secret settings.",
        "update_channel_bot" => {
            "Update a channel bot label or public app id. Enter credentials in the UI."
        }
        "delete_channel_bot" => "Delete a channel bot and its routes.",
        "list_channel_routes" => "List conversation-to-agent channel routes.",
        "set_channel_route" => {
            "Create or update a bot conversation's agent route and \
                default/agent-initiated settings. Assistant chat keys cannot be \
                route agents."
        }
        "delete_channel_route" => "Delete a channel conversation's agent route.",
        "list_my_services" => "List connected services, including disabled services and is_active.",
        "set_service_enabled" => "Enable or disable a connected service without deleting it.",
        "delete_service" => "Delete a connected service and its stored credential.",
        "list_nodes" => "List credential nodes without secrets.",
        "delete_node" => "Delete a credential node and disconnect its session.",
        "list_approval_configs" => "List per-service approval settings.",
        "set_approval_mode" => "Set a connected service's approval requirement and mode.",
        "list_pending_approvals" => {
            "Read pending approvals. Decisions must be made in the NyxID UI."
        }
        _ => "Unknown account tool.",
    };
    format!(
        "{purpose} {} In Ask mode, requires this chat's account acknowledgement. Key \
                creation/rotation, credentials, approval decisions, org \
                administration and billing are available only in the UI.",
        if destructive(name) {
            "Destructive: Ask mode also requires a single-use action acknowledgement."
        } else {
            "Non-destructive."
        }
    )
}

pub fn virtual_service() -> McpToolService {
    McpToolService {
        service_id: "nyxid".into(),
        service_name: "NyxID account".into(),
        service_slug: "nyxid".into(),
        description: Some(
            "Manage your NyxID account with this chat's human acknowledgement.".into(),
        ),
        service_category: "internal".into(),
        source: McpToolSource::Internal,
        executable: true,
        is_generic_proxy: false,
        invalid_openapi_contract: false,
        recommended_skills: Vec::new(),
        recommended_skill_refs: None,
        skills_revision: None,
        proxy_operation_policy: None,
        durable_endpoint_metadata: Default::default(),
        endpoints: TOOL_NAMES
            .iter()
            .map(|name| McpToolEndpoint {
                endpoint_id: format!("nyxid__{name}"),
                name: (*name).into(),
                description: Some(description(name)),
                method: "POST".into(),
                request_content_type: Some("application/json".into()),
                request_body_required: true,
                request_body_schema: Some(schema(name)),
                ..Default::default()
            })
            .collect(),
    }
}

fn matches_schema(value: &Value, schema: &Value) -> bool {
    if let Some(values) = schema["enum"].as_array()
        && !values.contains(value)
    {
        return false;
    }
    let matches_type = |kind: &str| match kind {
        "null" => value.is_null(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_u64().is_some(),
        "array" => value.is_array(),
        _ => false,
    };
    let valid_type = schema["type"]
        .as_str()
        .map(matches_type)
        .unwrap_or_else(|| {
            schema["type"]
                .as_array()
                .is_some_and(|types| types.iter().any(|t| t.as_str().is_some_and(matches_type)))
        });
    if !valid_type {
        return false;
    }
    if let Some(text) = value.as_str() {
        let len = text.chars().count() as u64;
        if schema["minLength"].as_u64().is_some_and(|min| len < min)
            || schema["maxLength"].as_u64().is_some_and(|max| len > max)
        {
            return false;
        }
    }
    if let Some(number) = value.as_u64()
        && (schema["minimum"].as_u64().is_some_and(|min| number < min)
            || schema["maximum"].as_u64().is_some_and(|max| number > max))
    {
        return false;
    }
    if let Some(array) = value.as_array() {
        return array.len() <= schema["maxItems"].as_u64().unwrap_or(100) as usize
            && array.iter().all(|v| matches_schema(v, &schema["items"]));
    }
    true
}

pub fn validate_arguments(name: &str, args: &Value) -> AppResult<()> {
    if !TOOL_NAMES.contains(&name) {
        return Err(AppError::NotFound("Account tool not found".into()));
    }
    let schema = schema(name);
    let valid = args.as_object().is_some_and(|args| {
        args.iter().all(|(key, value)| {
            schema["properties"]
                .get(key)
                .is_some_and(|s| matches_schema(value, s))
        }) && schema["required"].as_array().is_some_and(|fields| {
            fields
                .iter()
                .all(|field| args.contains_key(field.as_str().unwrap_or_default()))
        })
    });
    if !valid {
        return Err(AppError::ValidationError(
            "Invalid account tool arguments".into(),
        ));
    }
    Ok(())
}

pub struct AccountTools<'a> {
    pub db: &'a Database,
    pub keys: &'a EncryptionKeys,
    pub http: &'a reqwest::Client,
    pub config: &'a AppConfig,
    pub token_exchange_cache: &'a Arc<TokenExchangeCache>,
    pub node_manager: &'a NodeWsManager,
}

pub struct ToolResult {
    pub value: Value,
    pub is_error: bool,
}
impl std::fmt::Debug for ToolResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountToolResult { [REDACTED] }")
    }
}

pub fn error_result(error: AppError) -> ToolResult {
    let body = error.response_body();
    let message = match error {
        AppError::NotFound(_) | AppError::NodeNotFound(_) | AppError::ChannelBotNotFound(_) => {
            "Resource not found."
        }
        AppError::ValidationError(_) => {
            "Invalid account tool arguments or target. Assistant chat keys \
                cannot be modified or used as route agents."
        }
        AppError::Forbidden(_) | AppError::Unauthorized(_) => {
            "This operation requires a conversation key and human acknowledgement."
        }
        AppError::Conflict(_) => {
            "The resource changed. Inspect it and request a new acknowledgement."
        }
        _ => "The account operation could not be completed. Review it in the NyxID UI.",
    };
    ToolResult {
        is_error: true,
        value: json!({"error": body.error, "error_code": body.error_code, "message": message}),
    }
}

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or_default()
}
fn short(value: &str) -> String {
    value.chars().take(200).collect()
}
fn bounded_list(mut items: Vec<Value>) -> Value {
    let total = items.len();
    items.truncate(MAX_ITEMS);
    while serde_json::to_vec(&items).is_ok_and(|bytes| bytes.len() > MAX_RESULT_BYTES) {
        items.pop();
    }
    json!({"items": items, "total": total, "truncated": total > items.len()})
}
fn key_view(key: &ApiKey) -> Value {
    json!({"id": key.id, "name": short(&key.name), "key_prefix": key.key_prefix,
        "platform": key.platform, "description": key.description.as_deref().map(short),
        "scopes": key.scopes, "allow_all_services": key.allow_all_services,
        "allow_auto_connected_services": key.allow_auto_connected_services,
        "allow_all_nodes": key.allow_all_nodes,
        "allowed_service_ids": key.allowed_service_ids.iter().take(100).collect::<Vec<_>>(),
        "allowed_node_ids": key.allowed_node_ids.iter().take(100).collect::<Vec<_>>(),
        "rate_limit_per_second": key.rate_limit_per_second, "rate_limit_burst": key.rate_limit_burst,
        "has_callback_url": key.callback_url.is_some(), "is_active": key.is_active})
}
fn bot_view(bot: &ChannelBot) -> Value {
    json!({"id": bot.id, "label": short(&bot.label), "platform": bot.platform,
        "is_active": bot.is_active, "status": bot.status, "webhook_registered": bot.webhook_registered})
}
fn route_view(row: &ChannelConversation) -> Value {
    json!({"id": row.id, "channel_bot_id": row.channel_bot_id,
        "platform": row.platform, "platform_conversation_id": short(&row.platform_conversation_id),
        "agent_api_key_id": row.agent_api_key_id, "default_agent": row.default_agent,
        "allow_agent_initiated": row.allow_agent_initiated, "is_active": row.is_active})
}

impl AccountTools<'_> {
    pub async fn execute(&self, auth: &AuthUser, tool_name: &str, args: &Value) -> ToolResult {
        let user = auth.user_id.to_string();
        let chat = if auth.auth_method == AuthMethod::ApiKey {
            acks::for_key(self.db, &user, auth.api_key_id.as_deref()).await
        } else {
            Ok(None)
        };
        let access_mode = chat
            .as_ref()
            .ok()
            .and_then(|chat| chat.as_ref())
            .map(|chat| chat.access_mode);
        let conversation_id = chat
            .as_ref()
            .ok()
            .and_then(|chat| chat.as_ref())
            .map(|chat| chat.conversation_id.clone());
        let result = match chat {
            Ok(Some(chat)) => self.execute_authorized(&chat, auth, tool_name, args).await,
            Ok(None) => Err(AppError::Forbidden("Conversation key required".into())),
            Err(error) => Err(error),
        };
        let result = result.unwrap_or_else(error_result);
        // Never audit arguments, output, URLs, names or raw service-layer errors.
        let actor = audit_service::AuditActor::from_auth_user(auth);
        let target = ["api_key_id", "bot_id", "route_id", "service_id", "node_id"]
            .iter()
            .find_map(|field| args[*field].as_str())
            .filter(|id| UuidSafe::valid(id));
        let acknowledgement = args["acknowledgement_id"]
            .as_str()
            .filter(|id| UuidSafe::valid(id));
        let _ = audit_service::log_actor_event(
            self.db.clone(),
            &actor,
            "assistant_account_tool_call",
            Some(
                json!({"api_key_id": auth.api_key_id, "conversation_id": conversation_id,
                "tool_name": if tool_name.strip_prefix("nyxid__")
                    .is_some_and(|name| TOOL_NAMES.contains(&name)) {
                    tool_name
                } else {
                    "unknown"
                },
                "access_mode": access_mode,
                "target_id": target,
                "outcome": if result.is_error {"refused"} else {"success"},
                "acknowledgement_id": acknowledgement}),
            ),
        )
        .await;
        result
    }

    async fn execute_authorized(
        &self,
        chat: &acks::ChatAuthority,
        auth: &AuthUser,
        tool_name: &str,
        args: &Value,
    ) -> AppResult<ToolResult> {
        let name = tool_name.strip_prefix("nyxid__").unwrap_or_default();
        validate_arguments(name, args)?;
        super::assistant_nyxagent::require_enabled(self.db, &chat.user_id).await?;
        if let Some(refusal) = acks::account_gate(self.db, chat).await? {
            return Ok(ToolResult {
                value: refusal,
                is_error: true,
            });
        }
        if destructive(name)
            && chat.access_mode != crate::models::assistant_conversation::AccessMode::Full
        {
            // Resolve ownership and a human-readable summary before requesting authority.
            let summary = self.action_summary(&chat.user_id, name, args).await?;
            if let Some(id) = args["acknowledgement_id"].as_str() {
                if !acks::consume_action(self.db, chat, id, tool_name, args).await? {
                    return Ok(ToolResult {
                        is_error: true,
                        value: json!({
                            "error": "acknowledgement_invalid", "kind": "action",
                            "instructions": "The acknowledgement is missing, expired, used, \
                                or does not match this action. Ask the user before requesting \
                                a new action card.",
                        }),
                    });
                }
            } else {
                let row = acks::request(
                    self.db,
                    chat,
                    acks::Request {
                        kind: "action",
                        service: None,
                        tool: Some(tool_name),
                        arguments: Some(args),
                        summary: &summary,
                    },
                )
                .await?;
                return Ok(ToolResult {
                    value: acks::refusal(&row),
                    is_error: true,
                });
            }
        }
        let value = Box::pin(self.dispatch(&chat.user_id, auth, name, args)).await?;
        Ok(ToolResult {
            value,
            is_error: false,
        })
    }

    async fn mutable_key(&self, user: &str, id: &str) -> AppResult<ApiKey> {
        let key = key_service::get_api_key(self.db, user, id).await?;
        if key.platform.as_deref() == Some(ASSISTANT_PLATFORM)
            || acks::for_key(self.db, user, Some(id)).await?.is_some()
        {
            return Err(AppError::ValidationError(
                "Assistant chat keys cannot be modified by chat tools".into(),
            ));
        }
        Ok(key)
    }

    async fn own_route(&self, user: &str, id: &str) -> AppResult<ChannelConversation> {
        self.db
            .collection::<ChannelConversation>(crate::models::channel_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id": id, "user_id": user, "is_active": true})
            .await?
            .ok_or_else(|| AppError::NotFound("Channel route not found".into()))
    }

    async fn action_summary(&self, user: &str, name: &str, args: &Value) -> AppResult<String> {
        Ok(match name {
            "delete_agent_key" => {
                let key = self.mutable_key(user, text(args, "api_key_id")).await?;
                format!(
                    "Delete agent key '{}' ({})",
                    short(&key.name),
                    key.key_prefix
                )
            }
            "unbind_agent_key_credential" => {
                let key = self.mutable_key(user, text(args, "api_key_id")).await?;
                agent_binding_service::get_binding(
                    self.db,
                    user,
                    &key.id,
                    text(args, "binding_id"),
                )
                .await?;
                format!(
                    "Remove credential binding from agent key '{}'",
                    short(&key.name)
                )
            }
            "delete_channel_bot" => {
                let bot =
                    channel_bot_service::get_bot_for_user(self.db, text(args, "bot_id"), user)
                        .await?;
                format!("Delete channel bot '{}'", short(&bot.label))
            }
            "delete_channel_route" => {
                let route = self.own_route(user, text(args, "route_id")).await?;
                format!(
                    "Delete channel route '{}'",
                    short(&route.platform_conversation_id)
                )
            }
            "delete_service" => {
                let service =
                    user_service_service::get_user_service(self.db, user, text(args, "service_id"))
                        .await?;
                format!("Delete connected service '{}'", short(&service.slug))
            }
            "delete_node" => {
                let node = node_service::get_node(self.db, user, text(args, "node_id")).await?;
                format!("Delete node '{}'", short(&node.name))
            }
            _ => return Err(AppError::NotFound("Account tool not found".into())),
        })
    }

    async fn dispatch(
        &self,
        user: &str,
        auth: &AuthUser,
        name: &str,
        args: &Value,
    ) -> AppResult<Value> {
        match name {
            "list_agent_keys" => Ok(bounded_list(
                key_service::list_api_keys(self.db, user)
                    .await?
                    .iter()
                    .map(key_view)
                    .collect(),
            )),
            "get_agent_key" => Ok(key_view(
                &key_service::get_api_key(self.db, user, text(args, "api_key_id")).await?,
            )),
            "update_agent_key" => {
                let key = self.mutable_key(user, text(args, "api_key_id")).await?;
                // A normal key cannot be relabelled as an assistant key by these tools.
                if args["platform"].as_str() == Some(ASSISTANT_PLATFORM) {
                    return Err(AppError::ValidationError(
                        "Assistant platform is managed by chat".into(),
                    ));
                }
                let mut service_ids = Vec::new();
                if let Some(slugs) = args["allowed_service_slugs"].as_array() {
                    for slug in slugs {
                        let service = user_service_service::find_by_slug(
                            self.db,
                            user,
                            slug.as_str().unwrap_or_default(),
                        )
                        .await?
                        .ok_or_else(|| AppError::NotFound("Connected service not found".into()))?;
                        service_ids.push(service.id);
                    }
                }
                let node_ids: Option<Vec<String>> =
                    args["allowed_node_ids"].as_array().map(|ids| {
                        ids.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    });
                let updated = key_service::update_api_key_scope_with_expected_state_version(
                    self.db,
                    user,
                    Some(user),
                    &key.id,
                    args["name"].as_str(),
                    args["description"].as_str(),
                    None,
                    args.get("allowed_service_slugs")
                        .map(|_| service_ids.as_slice()),
                    node_ids.as_deref(),
                    args["allow_all_services"].as_bool(),
                    args["allow_auto_connected_services"].as_bool(),
                    args["allow_all_nodes"].as_bool(),
                    args.get("rate_limit_per_second")
                        .map(|v| v.as_u64().map(|n| n as u32)),
                    args.get("rate_limit_burst")
                        .map(|v| v.as_u64().map(|n| n as u32)),
                    args.get("platform").map(Value::as_str),
                    args.get("callback_url").map(Value::as_str),
                    None,
                    Some(key.state_version),
                )
                .await?;
                Ok(key_view(&updated))
            }
            "delete_agent_key" => {
                let key = self.mutable_key(user, text(args, "api_key_id")).await?;
                key_service::delete_api_key(self.db, user, &key.id).await?;
                Ok(json!({"deleted": true}))
            }
            "list_agent_key_bindings" => {
                let bindings =
                    agent_binding_service::list_bindings(self.db, user, text(args, "api_key_id"))
                        .await?;
                Ok(bounded_list(
                    bindings
                        .iter()
                        .map(|b| {
                            json!({"id": b.id,
                    "user_service_id": b.user_service_id, "user_api_key_id": b.user_api_key_id})
                        })
                        .collect(),
                ))
            }
            "bind_agent_key_credential" => {
                let key = self.mutable_key(user, text(args, "api_key_id")).await?;
                let binding = agent_binding_service::create_binding_with_scope_authorization(
                    self.db,
                    user,
                    Some(user),
                    &key.id,
                    text(args, "user_service_id"),
                    text(args, "user_api_key_id"),
                )
                .await?;
                Ok(json!({
                    "id": binding.id,
                    "user_service_id": binding.user_service_id,
                    "user_api_key_id": binding.user_api_key_id,
                }))
            }
            "unbind_agent_key_credential" => {
                let key = self.mutable_key(user, text(args, "api_key_id")).await?;
                agent_binding_service::delete_binding_with_scope_authorization(
                    self.db,
                    user,
                    Some(user),
                    &key.id,
                    text(args, "binding_id"),
                )
                .await?;
                Ok(json!({"deleted": true}))
            }
            "list_channel_bots" => Ok(bounded_list(
                channel_bot_service::list_bots(self.db, user)
                    .await?
                    .iter()
                    .map(bot_view)
                    .collect(),
            )),
            "get_channel_bot" => Ok(bot_view(
                &channel_bot_service::get_bot_for_user(self.db, text(args, "bot_id"), user).await?,
            )),
            "update_channel_bot" | "delete_channel_bot" => {
                let bot =
                    channel_bot_service::get_bot_for_user(self.db, text(args, "bot_id"), user)
                        .await?;
                let adapter =
                    channel_adapters::resolve_adapter(&bot.platform, self.token_exchange_cache)?;
                if name == "delete_channel_bot" {
                    channel_bot_service::delete_bot(
                        self.db,
                        self.config,
                        self.http,
                        self.keys,
                        adapter.as_ref(),
                        &bot.id,
                        user,
                    )
                    .await?;
                    Ok(json!({"deleted": true}))
                } else {
                    let bot = channel_bot_service::update_bot(
                        self.db,
                        self.keys,
                        self.http,
                        adapter.as_ref(),
                        &bot.id,
                        user,
                        channel_bot_service::UpdateBotParams {
                            label: args["label"].as_str(),
                            app_id: args["app_id"].as_str(),
                            bot_token: None,
                            app_secret: None,
                            verification_token: None,
                            encrypt_key: channel_bot_service::SecretPatch::Unchanged,
                        },
                    )
                    .await?;
                    Ok(bot_view(&bot))
                }
            }
            "list_channel_routes" => {
                if let Some(bot) = args["bot_id"].as_str() {
                    channel_bot_service::get_bot_for_user(self.db, bot, user).await?;
                }
                Ok(bounded_list(
                    channel_routing_service::list_conversations(
                        self.db,
                        user,
                        args["bot_id"].as_str(),
                    )
                    .await?
                    .iter()
                    .map(route_view)
                    .collect(),
                ))
            }
            "set_channel_route" => {
                let key = self
                    .mutable_key(user, text(args, "agent_api_key_id"))
                    .await?;
                let row = if let Some(id) = args["route_id"].as_str() {
                    self.own_route(user, id).await?;
                    channel_routing_service::update_conversation(
                        self.db,
                        id,
                        user,
                        Some(&key.id),
                        args["default_agent"].as_bool(),
                        None,
                        args["allow_agent_initiated"].as_bool(),
                    )
                    .await?
                } else {
                    let bot =
                        channel_bot_service::get_bot_for_user(self.db, text(args, "bot_id"), user)
                            .await?;
                    let default = args["default_agent"].as_bool().unwrap_or(false);
                    let conversation = args["platform_conversation_id"]
                        .as_str()
                        .or(default.then_some("*"))
                        .ok_or_else(|| {
                            AppError::ValidationError("A bot conversation is required".into())
                        })?;
                    channel_routing_service::create_conversation(
                        self.db,
                        user,
                        Some(&bot.id),
                        &bot.platform,
                        conversation,
                        args["platform_conversation_type"]
                            .as_str()
                            .unwrap_or("private"),
                        None,
                        &key.id,
                        default,
                        args["allow_agent_initiated"].as_bool().unwrap_or(false),
                    )
                    .await?
                };
                Ok(route_view(&row))
            }
            "delete_channel_route" => {
                self.own_route(user, text(args, "route_id")).await?;
                channel_routing_service::delete_conversation(self.db, text(args, "route_id"), user)
                    .await?;
                Ok(json!({"deleted": true}))
            }
            "list_my_services" => {
                let providers = platform_key_service::load_providers(self.db).await?;
                let services =
                    unified_key_service::list_keys(self.db, self.keys, user, &providers).await?;
                Ok(bounded_list(
                    services
                        .iter()
                        .map(|s| {
                            json!({"id": s.id, "slug": s.slug,
                    "label": short(&s.label), "is_active": s.is_active, "status": s.status,
                    "api_key_id": s.api_key_id, "auto_connected": s.auto_connected})
                        })
                        .collect(),
                ))
            }
            "set_service_enabled" => {
                user_service_service::update_user_service(
                    self.db,
                    user,
                    user,
                    text(args, "service_id"),
                    None,
                    None,
                    None,
                    None,
                    args["enabled"].as_bool(),
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
                Ok(json!({"id": args["service_id"], "is_active": args["enabled"]}))
            }
            "delete_service" => {
                let view = unified_key_service::get_key(
                    self.db,
                    self.keys,
                    user,
                    text(args, "service_id"),
                )
                .await?;
                if view.auto_connected {
                    return Err(AppError::Forbidden(
                        "Auto-connected services cannot be deleted".into(),
                    ));
                }
                let actor = audit_service::AuditActor::from_auth_user(auth);
                unified_key_service::disconnect_credentials(
                    self.db,
                    self.keys,
                    user,
                    &actor,
                    unified_key_service::DisconnectTarget::UserService(&view.id),
                    unified_key_service::DisconnectOptions {
                        cascade_grant: false,
                        grant_scope: None,
                    },
                )
                .await?;
                Ok(json!({"deleted": true}))
            }
            "list_nodes" => {
                let nodes = node_service::list_user_nodes(self.db, user).await?;
                Ok(bounded_list(
                    nodes
                        .iter()
                        .filter(|row| row.node.user_id == user)
                        .map(|row| {
                            json!({
                                "id": row.node.id,
                                "name": short(&row.node.name),
                                "status": row.node.status,
                            })
                        })
                        .collect(),
                ))
            }
            "delete_node" => {
                node_service::get_node(self.db, user, text(args, "node_id")).await?;
                node_service::delete_node(self.db, user, text(args, "node_id")).await?;
                self.node_manager
                    .disconnect_connection(text(args, "node_id"), 1000, "Node deleted")
                    .await;
                Ok(json!({"deleted": true}))
            }
            "list_approval_configs" => {
                let rows = approval_service::list_service_approval_configs(self.db, user).await?;
                Ok(bounded_list(
                    rows.iter()
                        .map(|row| {
                            json!({
                                "service_id": row.service_id,
                                "service_name": short(&row.service_name),
                                "approval_required": row.approval_required,
                                "approval_mode": row.approval_mode,
                            })
                        })
                        .collect(),
                ))
            }
            "set_approval_mode" => {
                let service =
                    user_service_service::get_user_service(self.db, user, text(args, "service_id"))
                        .await?;
                let mode = serde_json::from_value(args["approval_mode"].clone())
                    .map_err(|_| AppError::ValidationError("Invalid approval mode".into()))?;
                let id = service.catalog_service_id.as_deref().unwrap_or(&service.id);
                let row = approval_service::set_service_approval_config(
                    self.db,
                    user,
                    id,
                    &service.slug,
                    args["approval_required"].as_bool(),
                    Some(&mode),
                    None,
                    None,
                )
                .await?;
                Ok(json!({
                    "service_id": row.service_id,
                    "approval_required": row.approval_required,
                    "approval_mode": row.approval_mode,
                }))
            }
            "list_pending_approvals" => {
                let (rows, total) =
                    approval_service::list_requests(self.db, user, &[], &["pending"], 1, 100)
                        .await?;
                Ok(
                    json!({"total": total, "items": rows.iter().map(|row| json!({"id": row.id,
                    "service_id": row.service_id, "service_name": short(&row.service_name),
                    "status": row.status, "created_at": row.created_at.to_rfc3339()})).collect::<Vec<_>>()}),
                )
            }
            _ => Err(AppError::NotFound("Account tool not found".into())),
        }
    }
}

struct UuidSafe;
impl UuidSafe {
    fn valid(id: &str) -> bool {
        uuid::Uuid::parse_str(id).is_ok()
    }
}
