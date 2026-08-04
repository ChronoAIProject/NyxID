use std::collections::HashSet;

use crate::errors::{AppError, AppResult};
use crate::services::content_type::{
    is_binary_content_type, is_json_content_type, normalize_content_type,
    schema_contains_binary_field, schema_is_binary,
};

/// A single endpoint parsed from an OpenAPI/Swagger specification.
pub struct ParsedEndpoint {
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub request_content_type: Option<String>,
    pub request_body_required: bool,
}

#[derive(Default)]
struct ParsedRequestBody {
    content_type: Option<String>,
    schema: Option<serde_json::Value>,
    required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Swagger2FormBodyKind {
    FileUpload,
    FormFields,
}

/// Parse endpoints from a pre-fetched OpenAPI/Swagger spec JSON value.
///
/// Use this when the spec has already been fetched through a hardened path
/// (e.g., `api_docs_service::fetch_spec_json`).
pub fn parse_openapi_spec_value(spec: &serde_json::Value) -> AppResult<Vec<ParsedEndpoint>> {
    let is_openapi3 = spec.get("openapi").is_some();
    let is_swagger2 = spec.get("swagger").is_some();

    if !is_openapi3 && !is_swagger2 {
        return Err(AppError::BadRequest(
            "Spec must contain an 'openapi' or 'swagger' key".to_string(),
        ));
    }

    parse_endpoints_from_spec(spec, is_openapi3)
}

/// Fetch and parse an OpenAPI 3.x or Swagger 2.0 spec from a URL.
///
/// For each path+operation, extracts the operationId (or generates one from
/// method+path), summary/description, parameters, and requestBody schema.
pub async fn parse_openapi_spec(
    client: &reqwest::Client,
    url: &str,
) -> AppResult<Vec<ParsedEndpoint>> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to fetch OpenAPI spec: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "OpenAPI spec returned HTTP {}",
            resp.status()
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read OpenAPI spec body: {e}")))?;

    let spec: serde_json::Value = if body.trim_start().starts_with('{') {
        serde_json::from_str(&body)
            .map_err(|e| AppError::BadRequest(format!("Invalid JSON in OpenAPI spec: {e}")))?
    } else {
        // Try YAML parsing via serde_json (only JSON supported for now)
        return Err(AppError::BadRequest(
            "Only JSON OpenAPI specs are supported".to_string(),
        ));
    };

    // Determine spec version
    let is_openapi3 = spec.get("openapi").is_some();
    let is_swagger2 = spec.get("swagger").is_some();

    if !is_openapi3 && !is_swagger2 {
        return Err(AppError::BadRequest(
            "Spec must contain an 'openapi' or 'swagger' key".to_string(),
        ));
    }

    parse_endpoints_from_spec(&spec, is_openapi3)
}

fn parse_endpoints_from_spec(
    spec: &serde_json::Value,
    is_openapi3: bool,
) -> AppResult<Vec<ParsedEndpoint>> {
    let paths = spec
        .get("paths")
        .and_then(|p| p.as_object())
        .ok_or_else(|| AppError::BadRequest("Spec missing 'paths' object".to_string()))?;

    let http_methods = ["get", "post", "put", "delete", "patch"];
    let mut endpoints = Vec::new();

    for (path, path_item) in paths {
        let Some(path_obj) = path_item.as_object() else {
            continue;
        };

        for method in &http_methods {
            let Some(operation) = path_obj.get(*method) else {
                continue;
            };

            let name = extract_name(operation, method, path);
            let description = extract_description(operation);
            let parameters = extract_parameters_with_spec(operation, path_obj, spec);
            let request_body = if is_openapi3 {
                extract_request_body_openapi3_with_spec(operation, spec)
            } else {
                extract_request_body_swagger2(operation, path_obj, spec)
            };

            endpoints.push(ParsedEndpoint {
                name,
                description,
                method: method.to_uppercase(),
                path: path.clone(),
                parameters,
                request_body_schema: request_body.schema,
                request_content_type: request_body.content_type,
                request_body_required: request_body.required,
            });
        }
    }

    Ok(endpoints)
}

/// Extract or generate a tool-safe name from the operation.
fn extract_name(operation: &serde_json::Value, method: &str, path: &str) -> String {
    if let Some(id) = operation.get("operationId").and_then(|v| v.as_str()) {
        sanitize_name(id)
    } else {
        generate_name(method, path)
    }
}

/// Generate a name from method + path: e.g. GET /users/{id} -> get_users_by_id
fn generate_name(method: &str, path: &str) -> String {
    let path_part: String = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                format!("by_{}", &segment[1..segment.len() - 1])
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("_");

    let raw = format!("{}_{}", method.to_lowercase(), path_part);
    sanitize_name(&raw)
}

/// Sanitize a string into a valid MCP tool name: ^[a-z][a-z0-9_]*$
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();

    // Ensure starts with a letter
    let trimmed = cleaned.trim_start_matches('_');
    if trimmed.is_empty() {
        return "unnamed_endpoint".to_string();
    }

    let first = trimmed.chars().next().unwrap();
    if first.is_ascii_digit() {
        format!("op_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Extract description by combining summary and description fields.
///
/// When both `summary` and `description` are present, they are joined with
/// a newline so the MCP tool receives the full context. Falls back to
/// whichever field exists.
fn extract_description(operation: &serde_json::Value) -> Option<String> {
    let summary = operation.get("summary").and_then(|v| v.as_str());
    let description = operation.get("description").and_then(|v| v.as_str());

    match (summary, description) {
        (Some(s), Some(d)) => Some(format!("{s}\n\n{d}")),
        (Some(s), None) => Some(s.to_string()),
        (None, Some(d)) => Some(d.to_string()),
        (None, None) => None,
    }
}

/// Extract parameters from both operation-level and path-level.
#[cfg(test)]
fn extract_parameters(
    operation: &serde_json::Value,
    path_obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    extract_parameters_with_spec(operation, path_obj, operation)
}

fn extract_parameters_with_spec(
    operation: &serde_json::Value,
    path_obj: &serde_json::Map<String, serde_json::Value>,
    spec: &serde_json::Value,
) -> Option<serde_json::Value> {
    let mut all_params = Vec::new();

    // Path-level parameters
    if let Some(path_params) = path_obj.get("parameters").and_then(|v| v.as_array()) {
        for p in path_params {
            if let Some(param) = resolve_parameter_refs(spec, p)
                && should_include_parameter(&param)
            {
                merge_parameter(&mut all_params, param);
            }
        }
    }

    // Operation-level parameters (override path-level by name+in)
    if let Some(op_params) = operation.get("parameters").and_then(|v| v.as_array()) {
        for p in op_params {
            if let Some(param) = resolve_parameter_refs(spec, p)
                && should_include_parameter(&param)
            {
                merge_parameter(&mut all_params, param);
            }
        }
    }

    if all_params.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(all_params))
    }
}

fn should_include_parameter(param: &serde_json::Value) -> bool {
    matches!(
        param.get("in").and_then(|v| v.as_str()),
        Some("path" | "query" | "header" | "cookie")
    ) && param
        .get("name")
        .and_then(|v| v.as_str())
        .is_some_and(|name| !name.is_empty())
}

fn merge_parameter(params: &mut Vec<serde_json::Value>, param: serde_json::Value) {
    if let Some((name, location)) = parameter_identity(&param) {
        params.retain(|existing| {
            parameter_identity(existing) != Some((name.clone(), location.clone()))
        });
    }
    params.push(param);
}

/// Extract requestBody schema for OpenAPI 3.x.
#[cfg(test)]
fn extract_request_body_openapi3(operation: &serde_json::Value) -> ParsedRequestBody {
    extract_request_body_openapi3_with_spec(operation, operation)
}

fn extract_request_body_openapi3_with_spec(
    operation: &serde_json::Value,
    spec: &serde_json::Value,
) -> ParsedRequestBody {
    let Some(request_body) = operation.get("requestBody") else {
        return ParsedRequestBody::default();
    };

    let Some(request_body) = resolve_request_body_refs(spec, request_body) else {
        return ParsedRequestBody::default();
    };

    let Some(content) = request_body
        .get("content")
        .and_then(|content| content.as_object())
    else {
        return ParsedRequestBody::default();
    };

    let Some((content_type, media)) = select_openapi3_media(content, spec) else {
        return ParsedRequestBody::default();
    };

    ParsedRequestBody {
        content_type: Some(content_type.to_string()),
        schema: media
            .get("schema")
            .map(|schema| resolve_schema_refs(spec, schema)),
        required: request_body
            .get("required")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

/// Extract body parameter schema for Swagger 2.0.
fn extract_request_body_swagger2(
    operation: &serde_json::Value,
    path_obj: &serde_json::Map<String, serde_json::Value>,
    spec: &serde_json::Value,
) -> ParsedRequestBody {
    let find_body_param = |params: &serde_json::Value| -> Option<(serde_json::Value, bool)> {
        params.as_array()?.iter().find_map(|p| {
            let resolved = resolve_parameter_refs(spec, p)?;
            if resolved.get("in").and_then(|v| v.as_str()) == Some("body") {
                Some((
                    resolved
                        .get("schema")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                    resolved
                        .get("required")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                ))
            } else {
                None
            }
        })
    };

    let find_form_data_kind = |params: &serde_json::Value| -> Option<(Swagger2FormBodyKind, bool)> {
        let mut saw_form_fields = false;
        let mut saw_file_upload = false;
        let mut required = false;

        for param in params.as_array()? {
            let Some(param) = resolve_parameter_refs(spec, param) else {
                continue;
            };

            if param.get("in").and_then(|v| v.as_str()) != Some("formData") {
                continue;
            }

            required |= param
                .get("required")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if param.get("type").and_then(|v| v.as_str()) == Some("file") {
                saw_file_upload = true;
            } else {
                saw_form_fields = true;
            }
        }

        if saw_file_upload {
            Some((Swagger2FormBodyKind::FileUpload, required))
        } else if saw_form_fields {
            Some((Swagger2FormBodyKind::FormFields, required))
        } else {
            None
        }
    };

    // Check operation-level first, then path-level
    let (schema, required) = if let Some(params) = operation.get("parameters")
        && let Some((schema, required)) = find_body_param(params)
    {
        (Some(schema), required)
    } else if let Some(params) = path_obj.get("parameters")
        && let Some((schema, required)) = find_body_param(params)
    {
        (Some(schema), required)
    } else {
        (None, false)
    };

    let form_data = if let Some(params) = operation.get("parameters")
        && let Some(form_data) = find_form_data_kind(params)
    {
        Some(form_data)
    } else if let Some(params) = path_obj.get("parameters")
        && let Some(form_data) = find_form_data_kind(params)
    {
        Some(form_data)
    } else {
        None
    };

    let resolved_schema = schema
        .as_ref()
        .map(|schema| resolve_schema_refs(spec, schema));
    let form_data_kind = form_data.as_ref().map(|(kind, _)| *kind);
    let content_type =
        extract_swagger2_consumes(operation, spec, resolved_schema.as_ref(), form_data_kind);
    let body_required = if schema.is_some() {
        required
    } else {
        form_data.map(|(_, required)| required).unwrap_or(false)
    };

    ParsedRequestBody {
        content_type,
        schema: resolved_schema,
        required: body_required,
    }
}

fn select_openapi3_media<'a>(
    content: &'a serde_json::Map<String, serde_json::Value>,
    spec: &serde_json::Value,
) -> Option<(&'a str, &'a serde_json::Value)> {
    if let Some((content_type, media)) = content.iter().find(|(content_type, media)| {
        is_concrete_content_type(content_type) && is_upload_media(content_type, media, spec)
    }) {
        return Some((content_type.as_str(), media));
    }

    if let Some((content_type, media)) = content
        .iter()
        .find(|(content_type, media)| is_upload_media(content_type, media, spec))
    {
        return Some((content_type.as_str(), media));
    }

    if let Some((content_type, media)) = content.get_key_value("application/json") {
        return Some((content_type.as_str(), media));
    }

    if let Some((content_type, media)) = content
        .iter()
        .find(|(content_type, _)| is_json_content_type(content_type))
    {
        return Some((content_type.as_str(), media));
    }

    if let Some((content_type, media)) = content
        .iter()
        .find(|(content_type, _)| is_concrete_content_type(content_type))
    {
        return Some((content_type.as_str(), media));
    }

    content
        .get_key_value("*/*")
        .map(|(content_type, media)| (content_type.as_str(), media))
}

fn extract_swagger2_consumes(
    operation: &serde_json::Value,
    spec: &serde_json::Value,
    body_schema: Option<&serde_json::Value>,
    form_data_kind: Option<Swagger2FormBodyKind>,
) -> Option<String> {
    let prefers_binary = schema_is_binary(body_schema);

    operation
        .get("consumes")
        .and_then(|value| select_swagger2_content_type(value, prefers_binary, form_data_kind))
        .or_else(|| {
            spec.get("consumes").and_then(|value| {
                select_swagger2_content_type(value, prefers_binary, form_data_kind)
            })
        })
        .or_else(|| form_data_kind.map(default_swagger2_form_content_type))
        .or_else(|| prefers_binary.then(|| "application/octet-stream".to_string()))
}

fn select_swagger2_content_type(
    value: &serde_json::Value,
    prefers_binary: bool,
    form_data_kind: Option<Swagger2FormBodyKind>,
) -> Option<String> {
    let content_types = value.as_array()?;

    if let Some(kind) = form_data_kind
        && let Some(content_type) = select_swagger2_form_content_type(content_types, kind)
    {
        return Some(content_type);
    }

    if prefers_binary
        && let Some(content_type) = content_types.iter().find_map(|entry| {
            let content_type = entry.as_str()?;
            is_binary_content_type(content_type).then(|| content_type.to_string())
        })
    {
        return Some(content_type);
    }

    content_types
        .iter()
        .find_map(|entry| entry.as_str().map(ToString::to_string))
}

fn select_swagger2_form_content_type(
    content_types: &[serde_json::Value],
    kind: Swagger2FormBodyKind,
) -> Option<String> {
    let preferred = match kind {
        Swagger2FormBodyKind::FileUpload => ["multipart/form-data"].as_slice(),
        Swagger2FormBodyKind::FormFields => {
            ["application/x-www-form-urlencoded", "multipart/form-data"].as_slice()
        }
    };

    for preferred_type in preferred {
        if let Some(content_type) = content_types.iter().find_map(|entry| {
            let content_type = entry.as_str()?;
            (normalize_content_type(content_type) == *preferred_type)
                .then(|| content_type.to_string())
        }) {
            return Some(content_type);
        }
    }

    None
}

fn default_swagger2_form_content_type(kind: Swagger2FormBodyKind) -> String {
    match kind {
        Swagger2FormBodyKind::FileUpload => "multipart/form-data",
        Swagger2FormBodyKind::FormFields => "application/x-www-form-urlencoded",
    }
    .to_string()
}

fn is_concrete_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    normalized != "*/*" && !normalized.is_empty()
}

fn is_binary_media(
    content_type: &str,
    media: &serde_json::Value,
    spec: &serde_json::Value,
) -> bool {
    is_binary_content_type(content_type)
        || media
            .get("schema")
            .map(|schema| resolve_schema_refs(spec, schema))
            .as_ref()
            .is_some_and(|schema| schema_is_binary(Some(schema)))
}

fn is_upload_media(
    content_type: &str,
    media: &serde_json::Value,
    spec: &serde_json::Value,
) -> bool {
    let resolved_schema = media
        .get("schema")
        .map(|schema| resolve_schema_refs(spec, schema));

    is_binary_media(content_type, media, spec)
        || (normalize_content_type(content_type).starts_with("multipart/")
            && resolved_schema
                .as_ref()
                .is_some_and(|schema| schema_contains_binary_field(Some(schema))))
}

fn normalize_parameter_name(location: &str, name: &str) -> String {
    if location == "header" {
        name.trim().to_ascii_lowercase()
    } else {
        name.to_string()
    }
}

fn parameter_identity(param: &serde_json::Value) -> Option<(String, String)> {
    let name = param.get("name").and_then(|v| v.as_str())?;
    let location = param.get("in").and_then(|v| v.as_str())?;

    Some((
        normalize_parameter_name(location, name),
        location.to_string(),
    ))
}

fn resolve_local_ref<'a>(
    root: &'a serde_json::Value,
    value: &'a serde_json::Value,
) -> Option<&'a serde_json::Value> {
    let ref_str = value.get("$ref").and_then(|v| v.as_str())?;
    let pointer = ref_str.strip_prefix('#')?;
    root.pointer(pointer)
}

fn resolve_parameter_refs(
    root: &serde_json::Value,
    param: &serde_json::Value,
) -> Option<serde_json::Value> {
    let mut visited = HashSet::new();
    resolve_parameter_refs_inner(root, param, &mut visited)
}

fn resolve_request_body_refs(
    root: &serde_json::Value,
    request_body: &serde_json::Value,
) -> Option<serde_json::Value> {
    let mut visited = HashSet::new();
    resolve_request_body_refs_inner(root, request_body, &mut visited)
}

fn resolve_request_body_refs_inner(
    root: &serde_json::Value,
    request_body: &serde_json::Value,
    visited: &mut HashSet<String>,
) -> Option<serde_json::Value> {
    let mut resolved = if let Some(ref_str) = request_body.get("$ref").and_then(|v| v.as_str()) {
        if !visited.insert(ref_str.to_string()) {
            return Some(request_body.clone());
        }

        let target = resolve_local_ref(root, request_body)?;
        let mut expanded = resolve_request_body_refs_inner(root, target, visited)?;
        visited.remove(ref_str);

        if let (Some(request_body_obj), Some(expanded_obj)) =
            (request_body.as_object(), expanded.as_object_mut())
        {
            for (key, value) in request_body_obj {
                if key == "$ref" {
                    continue;
                }
                expanded_obj.insert(key.clone(), value.clone());
            }
        }

        expanded
    } else {
        request_body.clone()
    };

    if let Some(content) = resolved
        .get_mut("content")
        .and_then(|content| content.as_object_mut())
    {
        for media in content.values_mut() {
            if let Some(schema) = media.get_mut("schema") {
                *schema = resolve_schema_refs(root, schema);
            }
        }
    }

    Some(resolved)
}

fn resolve_parameter_refs_inner(
    root: &serde_json::Value,
    param: &serde_json::Value,
    visited: &mut HashSet<String>,
) -> Option<serde_json::Value> {
    let mut resolved = if let Some(ref_str) = param.get("$ref").and_then(|v| v.as_str()) {
        if !visited.insert(ref_str.to_string()) {
            return Some(param.clone());
        }

        let target = resolve_local_ref(root, param)?;
        let mut expanded = resolve_parameter_refs_inner(root, target, visited)?;
        visited.remove(ref_str);

        if let (Some(param_obj), Some(expanded_obj)) = (param.as_object(), expanded.as_object_mut())
        {
            for (key, value) in param_obj {
                if key == "$ref" {
                    continue;
                }
                expanded_obj.insert(key.clone(), value.clone());
            }
        }

        expanded
    } else {
        param.clone()
    };

    if let Some(schema) = resolved.get_mut("schema") {
        *schema = resolve_schema_refs(root, schema);
    }

    Some(resolved)
}

fn resolve_schema_refs(root: &serde_json::Value, schema: &serde_json::Value) -> serde_json::Value {
    let mut visited = HashSet::new();
    resolve_schema_refs_inner(root, schema, &mut visited)
}

fn resolve_schema_refs_inner(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    visited: &mut HashSet<String>,
) -> serde_json::Value {
    if let Some(ref_str) = schema.get("$ref").and_then(|v| v.as_str()) {
        if !visited.insert(ref_str.to_string()) {
            return schema.clone();
        }

        let resolved = resolve_local_ref(root, schema).unwrap_or(schema);
        let mut expanded = resolve_schema_refs_inner(root, resolved, visited);
        visited.remove(ref_str);

        if let (Some(schema_obj), Some(expanded_obj)) =
            (schema.as_object(), expanded.as_object_mut())
        {
            for (key, value) in schema_obj {
                if key == "$ref" {
                    continue;
                }
                expanded_obj.insert(key.clone(), resolve_schema_refs_inner(root, value, visited));
            }
        }

        return expanded;
    }

    let mut resolved = schema.clone();
    let Some(obj) = resolved.as_object_mut() else {
        return resolved;
    };

    if let Some(properties) = obj
        .get_mut("properties")
        .and_then(|value| value.as_object_mut())
    {
        for property_schema in properties.values_mut() {
            *property_schema = resolve_schema_refs_inner(root, property_schema, visited);
        }
    }

    if let Some(items) = obj.get_mut("items") {
        *items = resolve_schema_refs_inner(root, items, visited);
    }

    if let Some(additional_properties) = obj.get_mut("additionalProperties")
        && additional_properties.is_object()
    {
        *additional_properties = resolve_schema_refs_inner(root, additional_properties, visited);
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(variants) = obj.get_mut(key).and_then(|value| value.as_array_mut()) {
            for variant in variants {
                *variant = resolve_schema_refs_inner(root, variant, visited);
            }
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_name_simple() {
        assert_eq!(sanitize_name("getUser"), "getuser");
    }

    #[test]
    fn sanitize_name_replaces_special_chars() {
        assert_eq!(sanitize_name("get-user-by-id"), "get_user_by_id");
    }

    #[test]
    fn sanitize_name_strips_leading_underscores() {
        assert_eq!(sanitize_name("__hidden"), "hidden");
    }

    #[test]
    fn sanitize_name_digit_prefix() {
        assert_eq!(sanitize_name("123action"), "op_123action");
    }

    #[test]
    fn sanitize_name_empty_after_clean() {
        assert_eq!(sanitize_name("___"), "unnamed_endpoint");
    }

    #[test]
    fn generate_name_basic() {
        assert_eq!(generate_name("get", "/users"), "get_users");
    }

    #[test]
    fn generate_name_with_path_params() {
        assert_eq!(generate_name("get", "/users/{id}"), "get_users_by_id");
    }

    #[test]
    fn generate_name_nested_path() {
        assert_eq!(
            generate_name("post", "/users/{userId}/posts"),
            "post_users_by_userid_posts"
        );
    }

    #[test]
    fn extract_name_with_operation_id() {
        let op = serde_json::json!({"operationId": "listUsers"});
        assert_eq!(extract_name(&op, "get", "/users"), "listusers");
    }

    #[test]
    fn extract_name_without_operation_id() {
        let op = serde_json::json!({"summary": "Get users"});
        assert_eq!(extract_name(&op, "get", "/users"), "get_users");
    }

    #[test]
    fn extract_description_from_summary() {
        let op = serde_json::json!({"summary": "List all users"});
        assert_eq!(extract_description(&op), Some("List all users".to_string()));
    }

    #[test]
    fn extract_description_from_description_field() {
        let op = serde_json::json!({"description": "Detailed description"});
        assert_eq!(
            extract_description(&op),
            Some("Detailed description".to_string())
        );
    }

    #[test]
    fn extract_description_combines_summary_and_description() {
        let op = serde_json::json!({"summary": "Short", "description": "Long"});
        assert_eq!(extract_description(&op), Some("Short\n\nLong".to_string()));
    }

    #[test]
    fn extract_description_none() {
        let op = serde_json::json!({});
        assert_eq!(extract_description(&op), None);
    }

    #[test]
    fn extract_parameters_merges_path_and_op_level() {
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({
                "parameters": [{"name": "id", "in": "path"}],
                "get": {
                    "parameters": [{"name": "limit", "in": "query"}]
                }
            }))
            .unwrap();
        let operation = &path_obj["get"];
        let params = extract_parameters(operation, &path_obj);
        let arr = params.unwrap();
        let arr = arr.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn extract_parameters_none_when_empty() {
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({"get": {}})).unwrap();
        let operation = &path_obj["get"];
        let params = extract_parameters(operation, &path_obj);
        assert!(params.is_none());
    }

    #[test]
    fn extract_parameters_excludes_body_and_form_data_params() {
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({
                "post": {
                    "parameters": [
                        {"in": "query", "name": "limit"},
                        {"in": "body", "name": "payload", "schema": {"type": "object"}},
                        {"in": "formData", "name": "file", "type": "file"},
                        {"in": "path", "name": "id", "required": true}
                    ]
                }
            }))
            .unwrap();
        let operation = &path_obj["post"];
        let params = extract_parameters(operation, &path_obj).unwrap();
        let arr = params.as_array().unwrap();

        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "limit");
        assert_eq!(arr[1]["name"], "id");
    }

    #[test]
    fn extract_parameters_resolves_refs_and_operation_overrides_path_level() {
        let spec = serde_json::json!({
            "openapi": "3.0.0",
            "components": {
                "parameters": {
                    "LimitPath": {
                        "name": "limit",
                        "in": "query",
                        "required": false,
                        "schema": { "type": "integer" }
                    },
                    "LimitOp": {
                        "name": "limit",
                        "in": "query",
                        "required": true,
                        "schema": { "$ref": "#/components/schemas/LimitType" }
                    }
                },
                "schemas": {
                    "LimitType": {
                        "type": "integer",
                        "format": "int32"
                    }
                }
            }
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({
                "parameters": [
                    { "$ref": "#/components/parameters/LimitPath" }
                ],
                "get": {
                    "parameters": [
                        { "$ref": "#/components/parameters/LimitOp" }
                    ]
                }
            }))
            .unwrap();
        let operation = &path_obj["get"];
        let params = extract_parameters_with_spec(operation, &path_obj, &spec).unwrap();
        let arr = params.as_array().unwrap();

        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "limit");
        assert_eq!(arr[0]["required"], true);
        assert_eq!(arr[0]["schema"]["type"], "integer");
        assert_eq!(arr[0]["schema"]["format"], "int32");
    }

    #[test]
    fn extract_parameters_treats_header_overrides_case_insensitively() {
        let spec = serde_json::json!({
            "openapi": "3.0.0",
            "components": {
                "parameters": {
                    "HeaderPath": {
                        "name": "X-Api-Version",
                        "in": "header",
                        "required": false,
                        "schema": { "type": "string" }
                    },
                    "HeaderOp": {
                        "name": "x-api-version",
                        "in": "header",
                        "required": true,
                        "schema": { "type": "string", "enum": ["2025-01-01"] }
                    }
                }
            }
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({
                "parameters": [
                    { "$ref": "#/components/parameters/HeaderPath" }
                ],
                "get": {
                    "parameters": [
                        { "$ref": "#/components/parameters/HeaderOp" }
                    ]
                }
            }))
            .unwrap();
        let operation = &path_obj["get"];
        let params = extract_parameters_with_spec(operation, &path_obj, &spec).unwrap();
        let arr = params.as_array().unwrap();

        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "x-api-version");
        assert_eq!(arr[0]["in"], "header");
        assert_eq!(arr[0]["required"], true);
        assert_eq!(arr[0]["schema"]["enum"], serde_json::json!(["2025-01-01"]));
    }

    #[test]
    fn extract_parameters_excludes_referenced_body_and_form_data_params() {
        let spec = serde_json::json!({
            "swagger": "2.0",
            "parameters": {
                "Limit": {
                    "name": "limit",
                    "in": "query",
                    "type": "integer"
                },
                "Payload": {
                    "name": "payload",
                    "in": "body",
                    "schema": { "type": "object" }
                },
                "Upload": {
                    "name": "file",
                    "in": "formData",
                    "type": "file"
                }
            }
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({
                "post": {
                    "parameters": [
                        { "$ref": "#/parameters/Limit" },
                        { "$ref": "#/parameters/Payload" },
                        { "$ref": "#/parameters/Upload" }
                    ]
                }
            }))
            .unwrap();
        let operation = &path_obj["post"];
        let params = extract_parameters_with_spec(operation, &path_obj, &spec).unwrap();
        let arr = params.as_array().unwrap();

        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "limit");
    }

    #[test]
    fn extract_request_body_openapi3_found() {
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "application/json": {
                        "schema": {"type": "object"}
                    }
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("application/json"));
        assert!(body.schema.is_some());
        assert!(!body.required);
        assert_eq!(body.schema.as_ref().unwrap()["type"], "object");
    }

    #[test]
    fn extract_request_body_openapi3_preserves_required_flag() {
        let op = serde_json::json!({
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": {"type": "object"}
                    }
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("application/json"));
        assert!(body.required);
    }

    #[test]
    fn extract_request_body_openapi3_resolves_request_body_and_schema_refs() {
        let spec = serde_json::json!({
            "openapi": "3.0.0",
            "components": {
                "requestBodies": {
                    "SkillUpload": {
                        "required": true,
                        "content": {
                            "application/zip": {
                                "schema": {
                                    "$ref": "#/components/schemas/SkillArchive"
                                }
                            }
                        }
                    }
                },
                "schemas": {
                    "SkillArchive": {
                        "type": "string",
                        "format": "binary"
                    }
                }
            }
        });
        let op = serde_json::json!({
            "requestBody": {
                "$ref": "#/components/requestBodies/SkillUpload"
            }
        });

        let body = extract_request_body_openapi3_with_spec(&op, &spec);
        assert_eq!(body.content_type.as_deref(), Some("application/zip"));
        assert!(body.required);
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_openapi3_resolves_chained_request_body_refs() {
        let spec = serde_json::json!({
            "openapi": "3.0.0",
            "components": {
                "requestBodies": {
                    "SkillUploadAlias": {
                        "$ref": "#/components/requestBodies/SkillUpload"
                    },
                    "SkillUpload": {
                        "required": true,
                        "content": {
                            "application/zip": {
                                "schema": {
                                    "$ref": "#/components/schemas/SkillArchive"
                                }
                            }
                        }
                    }
                },
                "schemas": {
                    "SkillArchive": {
                        "type": "string",
                        "format": "binary"
                    }
                }
            }
        });
        let op = serde_json::json!({
            "requestBody": {
                "$ref": "#/components/requestBodies/SkillUploadAlias"
            }
        });

        let body = extract_request_body_openapi3_with_spec(&op, &spec);
        assert_eq!(body.content_type.as_deref(), Some("application/zip"));
        assert!(body.required);
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_openapi3_prefers_binary_schema_refs_over_json() {
        let spec = serde_json::json!({
            "openapi": "3.0.0",
            "components": {
                "schemas": {
                    "SkillArchive": {
                        "type": "string",
                        "format": "binary"
                    }
                }
            }
        });
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object"
                        }
                    },
                    "application/zip": {
                        "schema": {
                            "$ref": "#/components/schemas/SkillArchive"
                        }
                    }
                }
            }
        });

        let body = extract_request_body_openapi3_with_spec(&op, &spec);
        assert_eq!(body.content_type.as_deref(), Some("application/zip"));
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_openapi3_uses_non_json_content_when_json_absent() {
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "application/zip": {
                        "schema": {
                            "type": "string",
                            "format": "binary"
                        }
                    }
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("application/zip"));
        assert!(body.schema.is_some());
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_openapi3_keeps_content_type_without_schema() {
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "application/zip": {}
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("application/zip"));
        assert!(body.schema.is_none());
    }

    #[test]
    fn extract_request_body_openapi3_prefers_binary_media_over_json() {
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object"
                        }
                    },
                    "application/zip": {
                        "schema": {
                            "type": "string",
                            "format": "binary"
                        }
                    }
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("application/zip"));
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_openapi3_prefers_unknown_application_binary_media_over_json() {
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object"
                        }
                    },
                    "application/x-tar": {}
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("application/x-tar"));
        assert!(body.schema.is_none());
    }

    #[test]
    fn extract_request_body_openapi3_prefers_concrete_binary_media_over_wildcard() {
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "*/*": {
                        "schema": {
                            "type": "string",
                            "format": "binary"
                        }
                    },
                    "application/zip": {
                        "schema": {
                            "type": "string",
                            "format": "binary"
                        }
                    }
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("application/zip"));
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_openapi3_prefers_multipart_binary_file_uploads_over_json() {
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object"
                        }
                    },
                    "multipart/form-data": {
                        "schema": {
                            "type": "object",
                            "properties": {
                                "file": {
                                    "type": "string",
                                    "format": "binary"
                                }
                            }
                        }
                    }
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("multipart/form-data"));
        assert_eq!(
            body.schema.unwrap()["properties"]["file"]["format"],
            "binary"
        );
    }

    #[test]
    fn extract_request_body_openapi3_prefers_multipart_binary_additional_properties_over_json() {
        let op = serde_json::json!({
            "requestBody": {
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object"
                        }
                    },
                    "multipart/form-data": {
                        "schema": {
                            "type": "object",
                            "additionalProperties": {
                                "type": "string",
                                "format": "binary"
                            }
                        }
                    }
                }
            }
        });
        let body = extract_request_body_openapi3(&op);
        assert_eq!(body.content_type.as_deref(), Some("multipart/form-data"));
        assert_eq!(
            body.schema.unwrap()["additionalProperties"]["format"],
            "binary"
        );
    }

    #[test]
    fn extract_request_body_openapi3_missing() {
        let op = serde_json::json!({});
        let body = extract_request_body_openapi3(&op);
        assert!(body.content_type.is_none());
        assert!(body.schema.is_none());
    }

    #[test]
    fn extract_request_body_swagger2_from_body_param() {
        let op = serde_json::json!({
            "parameters": [
                {"in": "body", "schema": {"type": "object"}},
                {"in": "query", "name": "limit"}
            ]
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let spec = serde_json::json!({
            "consumes": ["application/json"]
        });
        let body = extract_request_body_swagger2(&op, &path_obj, &spec);
        assert_eq!(body.content_type.as_deref(), Some("application/json"));
        assert!(body.schema.is_some());
        assert!(!body.required);
        assert_eq!(body.schema.as_ref().unwrap()["type"], "object");
    }

    #[test]
    fn extract_request_body_swagger2_prefers_operation_consumes() {
        let op = serde_json::json!({
            "consumes": ["application/zip"],
            "parameters": [
                {"in": "body", "schema": {"type": "string", "format": "binary"}}
            ]
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let spec = serde_json::json!({
            "consumes": ["application/json"]
        });
        let body = extract_request_body_swagger2(&op, &path_obj, &spec);
        assert_eq!(body.content_type.as_deref(), Some("application/zip"));
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_swagger2_prefers_binary_consumes_for_binary_schema() {
        let op = serde_json::json!({
            "consumes": ["application/json", "application/octet-stream"],
            "parameters": [
                {"in": "body", "schema": {"type": "string", "format": "binary"}}
            ]
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let spec = serde_json::json!({});
        let body = extract_request_body_swagger2(&op, &path_obj, &spec);
        assert_eq!(
            body.content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_swagger2_resolves_binary_schema_refs_before_selecting_consumes() {
        let op = serde_json::json!({
            "parameters": [
                {"in": "body", "schema": {"$ref": "#/definitions/SkillArchive"}}
            ]
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let spec = serde_json::json!({
            "consumes": ["application/json", "application/octet-stream"],
            "definitions": {
                "SkillArchive": {
                    "type": "string",
                    "format": "binary"
                }
            }
        });
        let body = extract_request_body_swagger2(&op, &path_obj, &spec);
        assert_eq!(
            body.content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_swagger2_resolves_referenced_body_parameters() {
        let op = serde_json::json!({
            "parameters": [
                { "$ref": "#/parameters/UploadBody" }
            ]
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let spec = serde_json::json!({
            "consumes": ["application/json", "application/octet-stream"],
            "parameters": {
                "UploadBody": {
                    "name": "archive",
                    "in": "body",
                    "required": true,
                    "schema": { "$ref": "#/definitions/SkillArchive" }
                }
            },
            "definitions": {
                "SkillArchive": {
                    "type": "string",
                    "format": "binary"
                }
            }
        });
        let body = extract_request_body_swagger2(&op, &path_obj, &spec);
        assert_eq!(
            body.content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert!(body.required);
        assert_eq!(body.schema.unwrap()["format"], "binary");
    }

    #[test]
    fn extract_request_body_swagger2_detects_referenced_file_form_data_uploads() {
        let op = serde_json::json!({
            "consumes": ["application/json", "multipart/form-data"],
            "parameters": [
                { "$ref": "#/parameters/UploadFile" }
            ]
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let spec = serde_json::json!({
            "parameters": {
                "UploadFile": {
                    "name": "file",
                    "in": "formData",
                    "type": "file",
                    "required": true
                }
            }
        });
        let body = extract_request_body_swagger2(&op, &path_obj, &spec);
        assert_eq!(body.content_type.as_deref(), Some("multipart/form-data"));
        assert!(body.schema.is_none());
        assert!(body.required);
    }

    #[test]
    fn extract_request_body_swagger2_detects_file_form_data_uploads() {
        let op = serde_json::json!({
            "consumes": ["application/json", "multipart/form-data"],
            "parameters": [
                {"in": "formData", "name": "file", "type": "file", "required": true}
            ]
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let spec = serde_json::json!({});
        let body = extract_request_body_swagger2(&op, &path_obj, &spec);
        assert_eq!(body.content_type.as_deref(), Some("multipart/form-data"));
        assert!(body.schema.is_none());
        assert!(body.required);
    }

    #[test]
    fn extract_request_body_swagger2_detects_urlencoded_form_data() {
        let op = serde_json::json!({
            "consumes": ["application/json", "application/x-www-form-urlencoded"],
            "parameters": [
                {"in": "formData", "name": "message", "type": "string", "required": true}
            ]
        });
        let path_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).unwrap();
        let spec = serde_json::json!({});
        let body = extract_request_body_swagger2(&op, &path_obj, &spec);
        assert_eq!(
            body.content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        assert!(body.schema.is_none());
        assert!(body.required);
    }

    // ---- is_concrete_content_type ----

    #[test]
    fn is_concrete_content_type_rejects_wildcard() {
        assert!(!is_concrete_content_type("*/*"));
    }

    #[test]
    fn is_concrete_content_type_rejects_empty() {
        assert!(!is_concrete_content_type(""));
    }

    #[test]
    fn is_concrete_content_type_accepts_json() {
        assert!(is_concrete_content_type("application/json"));
    }

    #[test]
    fn is_concrete_content_type_accepts_with_params() {
        assert!(is_concrete_content_type("application/json; charset=utf-8"));
    }

    #[test]
    fn is_concrete_content_type_accepts_octet_stream() {
        assert!(is_concrete_content_type("application/octet-stream"));
    }

    #[test]
    fn is_concrete_content_type_accepts_multipart() {
        assert!(is_concrete_content_type("multipart/form-data"));
    }

    // ---- is_binary_media ----

    #[test]
    fn is_binary_media_by_content_type() {
        let media = serde_json::json!({});
        let spec = serde_json::json!({});
        assert!(is_binary_media("application/octet-stream", &media, &spec));
        assert!(is_binary_media("image/png", &media, &spec));
    }

    #[test]
    fn is_binary_media_by_schema_format() {
        let media = serde_json::json!({
            "schema": { "type": "string", "format": "binary" }
        });
        let spec = serde_json::json!({});
        assert!(is_binary_media("application/json", &media, &spec));
    }

    #[test]
    fn is_binary_media_false_for_json_object() {
        let media = serde_json::json!({
            "schema": { "type": "object" }
        });
        let spec = serde_json::json!({});
        assert!(!is_binary_media("application/json", &media, &spec));
    }

    #[test]
    fn is_binary_media_resolves_schema_refs() {
        let spec = serde_json::json!({
            "components": {
                "schemas": {
                    "BinaryBlob": {
                        "type": "string",
                        "format": "binary"
                    }
                }
            }
        });
        let media = serde_json::json!({
            "schema": { "$ref": "#/components/schemas/BinaryBlob" }
        });
        assert!(is_binary_media("application/json", &media, &spec));
    }

    // ---- is_upload_media ----

    #[test]
    fn is_upload_media_detects_binary_content_type() {
        let media = serde_json::json!({});
        let spec = serde_json::json!({});
        assert!(is_upload_media("application/octet-stream", &media, &spec));
    }

    #[test]
    fn is_upload_media_detects_multipart_with_binary_field() {
        let media = serde_json::json!({
            "schema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "format": "binary" }
                }
            }
        });
        let spec = serde_json::json!({});
        assert!(is_upload_media("multipart/form-data", &media, &spec));
    }

    #[test]
    fn is_upload_media_false_for_multipart_without_binary() {
        let media = serde_json::json!({
            "schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                }
            }
        });
        let spec = serde_json::json!({});
        assert!(!is_upload_media("multipart/form-data", &media, &spec));
    }

    // ---- normalize_parameter_name ----

    #[test]
    fn normalize_parameter_name_lowercases_headers() {
        assert_eq!(normalize_parameter_name("header", "X-Api-Key"), "x-api-key");
    }

    #[test]
    fn normalize_parameter_name_trims_header_whitespace() {
        assert_eq!(
            normalize_parameter_name("header", "  X-Api-Key  "),
            "x-api-key"
        );
    }

    #[test]
    fn normalize_parameter_name_preserves_query_case() {
        assert_eq!(normalize_parameter_name("query", "pageSize"), "pageSize");
    }

    #[test]
    fn normalize_parameter_name_preserves_path_case() {
        assert_eq!(normalize_parameter_name("path", "userId"), "userId");
    }

    // ---- parameter_identity ----

    #[test]
    fn parameter_identity_extracts_name_and_location() {
        let param = serde_json::json!({"name": "limit", "in": "query"});
        let identity = parameter_identity(&param);
        assert_eq!(identity, Some(("limit".to_string(), "query".to_string())));
    }

    #[test]
    fn parameter_identity_normalizes_header_name() {
        let param = serde_json::json!({"name": "X-Request-Id", "in": "header"});
        let identity = parameter_identity(&param);
        assert_eq!(
            identity,
            Some(("x-request-id".to_string(), "header".to_string()))
        );
    }

    #[test]
    fn parameter_identity_returns_none_without_name() {
        let param = serde_json::json!({"in": "query"});
        assert!(parameter_identity(&param).is_none());
    }

    #[test]
    fn parameter_identity_returns_none_without_in() {
        let param = serde_json::json!({"name": "limit"});
        assert!(parameter_identity(&param).is_none());
    }

    // ---- should_include_parameter ----

    #[test]
    fn should_include_parameter_accepts_valid_locations() {
        for location in ["path", "query", "header", "cookie"] {
            let param = serde_json::json!({"name": "test", "in": location});
            assert!(
                should_include_parameter(&param),
                "should include parameter in '{location}'"
            );
        }
    }

    #[test]
    fn should_include_parameter_rejects_body_and_form_data() {
        for location in ["body", "formData"] {
            let param = serde_json::json!({"name": "test", "in": location});
            assert!(
                !should_include_parameter(&param),
                "should not include parameter in '{location}'"
            );
        }
    }

    #[test]
    fn should_include_parameter_rejects_empty_name() {
        let param = serde_json::json!({"name": "", "in": "query"});
        assert!(!should_include_parameter(&param));
    }

    #[test]
    fn should_include_parameter_rejects_missing_name() {
        let param = serde_json::json!({"in": "query"});
        assert!(!should_include_parameter(&param));
    }

    #[test]
    fn should_include_parameter_rejects_missing_in() {
        let param = serde_json::json!({"name": "limit"});
        assert!(!should_include_parameter(&param));
    }

    // ---- resolve_local_ref ----

    #[test]
    fn resolve_local_ref_follows_json_pointer() {
        let root = serde_json::json!({
            "components": {
                "schemas": {
                    "User": { "type": "object" }
                }
            }
        });
        let ref_value = serde_json::json!({"$ref": "#/components/schemas/User"});
        let resolved = resolve_local_ref(&root, &ref_value);
        assert_eq!(resolved.unwrap()["type"], "object");
    }

    #[test]
    fn resolve_local_ref_returns_none_for_missing_target() {
        let root = serde_json::json!({});
        let ref_value = serde_json::json!({"$ref": "#/components/schemas/Missing"});
        assert!(resolve_local_ref(&root, &ref_value).is_none());
    }

    #[test]
    fn resolve_local_ref_returns_none_for_non_local_ref() {
        let root = serde_json::json!({});
        let ref_value = serde_json::json!({"$ref": "https://example.com/schema.json"});
        assert!(resolve_local_ref(&root, &ref_value).is_none());
    }

    #[test]
    fn resolve_local_ref_returns_none_without_ref_key() {
        let root = serde_json::json!({});
        let value = serde_json::json!({"type": "object"});
        assert!(resolve_local_ref(&root, &value).is_none());
    }

    // ---- resolve_schema_refs ----

    #[test]
    fn resolve_schema_refs_expands_nested_properties() {
        let root = serde_json::json!({
            "components": {
                "schemas": {
                    "Address": {
                        "type": "object",
                        "properties": {
                            "street": { "type": "string" }
                        }
                    }
                }
            }
        });
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "address": { "$ref": "#/components/schemas/Address" }
            }
        });
        let resolved = resolve_schema_refs(&root, &schema);
        assert_eq!(
            resolved["properties"]["address"]["properties"]["street"]["type"],
            "string"
        );
    }

    #[test]
    fn resolve_schema_refs_handles_array_items() {
        let root = serde_json::json!({
            "components": {
                "schemas": {
                    "Item": { "type": "string" }
                }
            }
        });
        let schema = serde_json::json!({
            "type": "array",
            "items": { "$ref": "#/components/schemas/Item" }
        });
        let resolved = resolve_schema_refs(&root, &schema);
        assert_eq!(resolved["items"]["type"], "string");
    }

    #[test]
    fn resolve_schema_refs_handles_all_of() {
        let root = serde_json::json!({
            "components": {
                "schemas": {
                    "Base": { "type": "object", "properties": { "id": { "type": "string" } } }
                }
            }
        });
        let schema = serde_json::json!({
            "allOf": [
                { "$ref": "#/components/schemas/Base" },
                { "properties": { "name": { "type": "string" } } }
            ]
        });
        let resolved = resolve_schema_refs(&root, &schema);
        let all_of = resolved["allOf"].as_array().unwrap();
        assert_eq!(all_of[0]["properties"]["id"]["type"], "string");
    }

    #[test]
    fn resolve_schema_refs_handles_circular_refs() {
        let root = serde_json::json!({
            "components": {
                "schemas": {
                    "Node": {
                        "type": "object",
                        "properties": {
                            "children": {
                                "type": "array",
                                "items": { "$ref": "#/components/schemas/Node" }
                            }
                        }
                    }
                }
            }
        });
        let schema = serde_json::json!({"$ref": "#/components/schemas/Node"});
        // Should not stack overflow -- the cycle is broken by the visited set
        let resolved = resolve_schema_refs(&root, &schema);
        assert_eq!(resolved["type"], "object");
    }

    #[test]
    fn resolve_schema_refs_expands_additional_properties() {
        let root = serde_json::json!({
            "components": {
                "schemas": {
                    "Value": { "type": "string" }
                }
            }
        });
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": { "$ref": "#/components/schemas/Value" }
        });
        let resolved = resolve_schema_refs(&root, &schema);
        assert_eq!(resolved["additionalProperties"]["type"], "string");
    }

    // ---- default_swagger2_form_content_type ----

    #[test]
    fn default_swagger2_form_content_type_file_upload() {
        assert_eq!(
            default_swagger2_form_content_type(Swagger2FormBodyKind::FileUpload),
            "multipart/form-data"
        );
    }

    #[test]
    fn default_swagger2_form_content_type_form_fields() {
        assert_eq!(
            default_swagger2_form_content_type(Swagger2FormBodyKind::FormFields),
            "application/x-www-form-urlencoded"
        );
    }

    // ---- select_swagger2_form_content_type ----

    #[test]
    fn select_swagger2_form_content_type_file_upload_finds_multipart() {
        let types = vec![
            serde_json::json!("application/json"),
            serde_json::json!("multipart/form-data"),
        ];
        let result = select_swagger2_form_content_type(&types, Swagger2FormBodyKind::FileUpload);
        assert_eq!(result.as_deref(), Some("multipart/form-data"));
    }

    #[test]
    fn select_swagger2_form_content_type_form_fields_prefers_urlencoded() {
        let types = vec![
            serde_json::json!("multipart/form-data"),
            serde_json::json!("application/x-www-form-urlencoded"),
        ];
        let result = select_swagger2_form_content_type(&types, Swagger2FormBodyKind::FormFields);
        assert_eq!(result.as_deref(), Some("application/x-www-form-urlencoded"));
    }

    #[test]
    fn select_swagger2_form_content_type_form_fields_falls_back_to_multipart() {
        let types = vec![
            serde_json::json!("application/json"),
            serde_json::json!("multipart/form-data"),
        ];
        let result = select_swagger2_form_content_type(&types, Swagger2FormBodyKind::FormFields);
        assert_eq!(result.as_deref(), Some("multipart/form-data"));
    }

    #[test]
    fn select_swagger2_form_content_type_returns_none_when_no_match() {
        let types = vec![serde_json::json!("application/json")];
        let result = select_swagger2_form_content_type(&types, Swagger2FormBodyKind::FileUpload);
        assert!(result.is_none());
    }

    // ---- select_swagger2_content_type ----

    #[test]
    fn select_swagger2_content_type_prefers_binary_for_binary_schema() {
        let value = serde_json::json!(["application/json", "application/octet-stream"]);
        let result = select_swagger2_content_type(&value, true, None);
        assert_eq!(result.as_deref(), Some("application/octet-stream"));
    }

    #[test]
    fn select_swagger2_content_type_returns_first_when_not_binary() {
        let value = serde_json::json!(["application/json", "text/plain"]);
        let result = select_swagger2_content_type(&value, false, None);
        assert_eq!(result.as_deref(), Some("application/json"));
    }

    #[test]
    fn select_swagger2_content_type_returns_none_for_non_array() {
        let value = serde_json::json!("application/json");
        let result = select_swagger2_content_type(&value, false, None);
        assert!(result.is_none());
    }

    #[test]
    fn select_swagger2_content_type_form_data_takes_priority() {
        let value = serde_json::json!([
            "application/json",
            "multipart/form-data",
            "application/octet-stream"
        ]);
        let result =
            select_swagger2_content_type(&value, true, Some(Swagger2FormBodyKind::FileUpload));
        assert_eq!(result.as_deref(), Some("multipart/form-data"));
    }

    // ---- parse_openapi_spec_value ----

    #[test]
    fn parse_openapi_spec_value_rejects_non_spec_document() {
        let spec = serde_json::json!({"foo": "bar"});
        let result = parse_openapi_spec_value(&spec);
        assert!(result.is_err());
    }

    #[test]
    fn parse_openapi_spec_value_rejects_missing_paths() {
        let spec = serde_json::json!({"openapi": "3.0.0"});
        let result = parse_openapi_spec_value(&spec);
        assert!(result.is_err());
    }

    #[test]
    fn parse_openapi_spec_value_parses_minimal_openapi3() {
        let spec = serde_json::json!({
            "openapi": "3.0.0",
            "paths": {
                "/users": {
                    "get": {
                        "operationId": "listUsers",
                        "responses": { "200": {} }
                    }
                }
            }
        });
        let endpoints = parse_openapi_spec_value(&spec).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].name, "listusers");
        assert_eq!(endpoints[0].method, "GET");
        assert_eq!(endpoints[0].path, "/users");
    }

    #[test]
    fn parse_openapi_spec_value_parses_swagger2() {
        let spec = serde_json::json!({
            "swagger": "2.0",
            "paths": {
                "/items": {
                    "post": {
                        "summary": "Create item",
                        "parameters": [
                            {"in": "body", "name": "item", "schema": {"type": "object"}}
                        ]
                    }
                }
            }
        });
        let endpoints = parse_openapi_spec_value(&spec).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].method, "POST");
    }

    // ---- extract_swagger2_consumes ----

    #[test]
    fn extract_swagger2_consumes_falls_back_to_octet_stream_for_binary_schema() {
        let op = serde_json::json!({});
        let spec = serde_json::json!({});
        let schema = serde_json::json!({"type": "string", "format": "binary"});
        let result = extract_swagger2_consumes(&op, &spec, Some(&schema), None);
        assert_eq!(result.as_deref(), Some("application/octet-stream"));
    }

    #[test]
    fn extract_swagger2_consumes_falls_back_to_form_default_without_consumes() {
        let op = serde_json::json!({});
        let spec = serde_json::json!({});
        let result =
            extract_swagger2_consumes(&op, &spec, None, Some(Swagger2FormBodyKind::FileUpload));
        assert_eq!(result.as_deref(), Some("multipart/form-data"));
    }

    #[test]
    fn extract_swagger2_consumes_returns_none_without_schema_or_form() {
        let op = serde_json::json!({});
        let spec = serde_json::json!({});
        let result = extract_swagger2_consumes(&op, &spec, None, None);
        assert!(result.is_none());
    }

    // ---- merge_parameter ----

    #[test]
    fn merge_parameter_replaces_by_identity() {
        let mut params =
            vec![serde_json::json!({"name": "limit", "in": "query", "required": false})];
        let new_param = serde_json::json!({"name": "limit", "in": "query", "required": true});
        merge_parameter(&mut params, new_param);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0]["required"], true);
    }

    #[test]
    fn merge_parameter_appends_different_params() {
        let mut params = vec![serde_json::json!({"name": "limit", "in": "query"})];
        let new_param = serde_json::json!({"name": "offset", "in": "query"});
        merge_parameter(&mut params, new_param);
        assert_eq!(params.len(), 2);
    }
}
