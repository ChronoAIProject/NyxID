//! Compiled-catalog JSON adapters for Lark/Feishu IM. No caller-supplied
//! marker grants a payload transformer or a credential-bearing fetch.
use super::{catalog_spec_registry, catalog_spec_sync, mcp_service::McpToolEndpoint};
use crate::{
    errors::{AppError, AppResult},
    models::downstream_service::DownstreamService,
};
use serde::Serialize;
use serde_json::{Value, json};

pub mod image;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Message,
    Image,
}

/// Candidate matching denies untrusted image materialization, never grants it.
pub fn image_candidate(endpoint: &McpToolEndpoint) -> bool {
    endpoint.name == "im_image_create"
        || (endpoint.method.eq_ignore_ascii_case("POST")
            && matches!(
                endpoint.path.as_str(),
                "/im/v1/images" | "/open-apis/im/v1/images"
            ))
}

pub fn stored(catalog: Option<&DownstreamService>, endpoint: &McpToolEndpoint) -> Option<Kind> {
    let catalog = catalog?;
    if catalog.created_by != "system"
        || catalog.tools_manager_created
        || !matches!(
            catalog_spec_registry::spec_key_for_slug(&catalog.slug),
            Some("lark" | "lark-bot")
        )
    {
        return None;
    }
    let spec = catalog_spec_registry::spec_for_slug(&catalog.slug)?;
    let compiled = catalog_spec_sync::destination_endpoint_inputs(catalog, &spec)
        .ok()?
        .into_iter()
        .find(|e| e.name == endpoint.name)?;
    let mark = &spec["paths"][&compiled.path][compiled.method.to_ascii_lowercase()];
    let kind = if mark["x-nyxid-message-content"] == json!({"version":1}) {
        Kind::Message
    } else if mark["x-nyxid-managed-operation"] == json!({"kind":"lark_im_image","version":1}) {
        Kind::Image
    } else {
        return None;
    };
    let expected = McpToolEndpoint {
        name: compiled.name,
        method: compiled.method,
        path: compiled.path,
        target_id: compiled.target_id,
        parameters: compiled.parameters,
        request_body_schema: compiled.request_body_schema,
        request_content_type: compiled.request_content_type,
        request_body_required: compiled.request_body_required,
        async_operation: compiled.async_operation,
        ..Default::default()
    };
    (super::lark_attachment_tool::inputs(endpoint)
        == super::lark_attachment_tool::inputs(&expected))
    .then_some(kind)
}

pub fn kind(endpoint: &McpToolEndpoint) -> Option<Kind> {
    match endpoint.attachment_input {
        Some(super::lark_attachment_tool::Projection::Im(kind)) => Some(kind),
        _ => None,
    }
}

pub fn invalid() -> AppError {
    AppError::ValidationError("lark_message_content_invalid: provide exactly one of plain text with msg_type=text, or content as a JSON object or JSON-encoded object string".into())
}

#[derive(Clone)]
pub struct MessageMode {
    msg_type: &'static str,
    mode: &'static str,
}

/// Parsing a legacy string validates it without changing its bytes. In
/// particular, a literal backslash-n is never guessed to mean a newline.
pub fn normalize_message(body: &mut Value) -> AppResult<MessageMode> {
    let object = body.as_object_mut().ok_or_else(invalid)?;
    let msg_type = object
        .get("msg_type")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let diagnostic_type = match msg_type {
        "text" => "text",
        "post" => "post",
        "interactive" => "interactive",
        "image" => "image",
        "file" => "file",
        "audio" => "audio",
        "media" => "media",
        "sticker" => "sticker",
        "share_chat" => "share_chat",
        "share_user" => "share_user",
        _ => "other",
    };
    let (content, mode) = match (object.get("text"), object.get("content")) {
        (Some(Value::String(text)), None) if msg_type == "text" => {
            (json!({"text":text}).to_string(), "typed")
        }
        (None, Some(raw)) => {
            let parsed = match raw {
                Value::String(raw) => serde_json::from_str::<Value>(raw).map_err(|_| invalid())?,
                Value::Object(_) => raw.clone(),
                _ => return Err(invalid()),
            };
            if !parsed.is_object() || (msg_type == "text" && !parsed["text"].is_string()) {
                return Err(invalid());
            }
            (
                raw.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| parsed.to_string()),
                "raw",
            )
        }
        _ => return Err(invalid()),
    };
    object.remove("text");
    object.insert("content".into(), content.into());
    Ok(MessageMode {
        msg_type: diagnostic_type,
        mode,
    })
}

/// The legacy /mcp/config bridge forwards HTTP itself and cannot run server
/// adapters. Preserve its existing raw message contract; omit image upload.
pub fn legacy_message_contract(endpoint: &mut McpToolEndpoint) {
    if kind(endpoint) != Some(Kind::Message) {
        return;
    }
    if let Some(schema) = endpoint.request_body_schema.as_mut() {
        schema
            .as_object_mut()
            .expect("compiled schema")
            .remove("oneOf");
        schema["properties"]
            .as_object_mut()
            .expect("compiled properties")
            .remove("text");
        schema["properties"]["content"] = json!({"type":"string","description":"JSON-encoded message content string matching msg_type. Typed text is available through hosted NyxID MCP."});
        schema["required"]
            .as_array_mut()
            .expect("compiled required")
            .push(json!("content"));
    }
}

/// Never return provider prose: it may echo message content, signed URLs or
/// base64. A nonzero application code is an MCP failure even on HTTP 200.
pub fn response(
    kind: Kind,
    mode: Option<&MessageMode>,
    status: u16,
    text: &str,
) -> AppResult<String> {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    if kind == Kind::Message {
        if value["code"].as_i64() == Some(230001) {
            let mode = mode.ok_or_else(invalid)?;
            let diagnostic = json!({"error":"lark_message_rejected","provider_code":230001,
                "msg_type":mode.msg_type,"mode":mode.mode,"local_json_parsed":true,"request_sent":true});
            tracing::warn!(
                msg_type = mode.msg_type,
                mode = mode.mode,
                local_json_parsed = true,
                request_sent = true,
                provider_code = 230001,
                "Lark message rejected"
            );
            return Err(AppError::BadRequest(diagnostic.to_string()));
        }
        if !(200..300).contains(&status) || value["code"].as_i64().is_some_and(|code| code != 0) {
            return Err(AppError::BadRequest("lark_message_rejected: request_sent=true; check the sending account's permissions and message contract".into()));
        }
        return Ok(text.to_owned());
    }
    let key = value["data"]["image_key"]
        .as_str()
        .filter(|key| !key.is_empty() && key.len() <= 1024 && !key.chars().any(char::is_control));
    if !(200..300).contains(&status) || value["code"].as_i64() != Some(0) || key.is_none() {
        return Err(AppError::BadRequest("lark_image_upload_rejected: no usable image_key; check im:resource and the selected account's tenant authorization".into()));
    }
    Ok(json!({"image_key":key}).to_string())
}
