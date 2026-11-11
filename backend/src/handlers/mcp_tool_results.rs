//! Transport-only projection: keep receipt text and progress digest independent of refs.
use super::*;
use crate::services::assistant_tool_result_service as retained;

/// Only the exact chat credential participates; narrower bearer/relay authority
/// must not borrow a result issued under its parent key.
pub(super) fn chat(
    auth: &McpAuthContext,
) -> Option<&crate::services::assistant_acknowledgement_service::ChatAuthority> {
    if auth.auth_method != AuthMethod::ApiKey {
        return None;
    }
    auth.chat.as_ref().filter(|chat| {
        auth.api_key_id.as_deref() == Some(chat.api_key_id.as_str())
            && chat.turn_live
            && !chat.turn_stopped
            && (!chat.guest || !chat.is_orchestrator())
    })
}

pub(super) async fn project(
    state: &AppState,
    auth: &McpAuthContext,
    content: Vec<serde_json::Value>,
    is_error: bool,
) -> (Vec<serde_json::Value>, Option<ToolResultDigest>) {
    let Some(chat) = chat(auth) else {
        return (content, None);
    };
    if is_error || content.len() != 1 || content[0]["type"] != "text" {
        return (content, None);
    }
    let original = serde_json::json!({"content":content,"isError":false});
    if original.to_string().len() <= retained::BUDGET {
        return (content, None);
    }
    let digest = result_digest(&original);
    let text = content[0]["text"].as_str().unwrap_or_default();
    let value = match retained::retain(&state.db, &state.encryption_keys, chat, &auth.user_id, text)
        .await
    {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                code = error.error_code(),
                "Large tool result retention unavailable"
            );
            retained::unavailable()
        }
    };
    (
        vec![serde_json::json!({"type":"text","text":value.to_string()})],
        Some(digest),
    )
}

pub(super) async fn read(
    state: &AppState,
    auth: &McpAuthContext,
    arguments: &serde_json::Value,
    id: Option<serde_json::Value>,
) -> Response {
    let Some(chat) = chat(auth) else {
        return tool_result(
            id,
            "Tool result continuation requires its original live chat turn",
            true,
        );
    };
    let request: retained::ReadRequest = match serde_json::from_value(arguments.clone()) {
        Ok(value) => value,
        Err(_) => {
            return tool_result(
                id,
                "Use ref and a zero-based nonnegative integer part",
                true,
            );
        }
    };
    match retained::read(
        &state.db,
        &state.encryption_keys,
        chat,
        &auth.user_id,
        &request,
    )
    .await
    {
        Ok(mut value) => {
            let mut response = tool_result(id, &value.to_string(), false);
            // Random refs are transport locators, not business progress.
            value.as_object_mut().expect("part").remove("ref");
            response.extensions_mut().insert(result_digest(&value));
            response
        }
        Err(_) => tool_result(id, &retained::missing().to_string(), true),
    }
}

/// A part locator must not manufacture progress on identical business output.
/// Keep all other tools' legacy name/argument digest exactly as it was.
pub(super) fn progress_params(params: &serde_json::Value) -> serde_json::Value {
    let mut name = params["name"].as_str().unwrap_or_default();
    let mut args = params["arguments"].clone();
    if name == "nyx__call_tool" && args["tool_name"] == retained::TOOL_NAME {
        name = retained::TOOL_NAME;
        args = args["arguments_json"]
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .or_else(|| args.get("arguments").cloned())
            .unwrap_or_else(|| args.clone());
    }
    if name != retained::TOOL_NAME {
        return params.clone();
    }
    serde_json::json!({"name":name,"arguments":{"part":args["part"]}})
}
