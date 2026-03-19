use std::collections::HashSet;

use crate::errors::{AppError, AppResult};

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
            let parameters = extract_parameters_with_spec(operation, path_obj, &spec);
            let request_body = if is_openapi3 {
                extract_request_body_openapi3_with_spec(operation, &spec)
            } else {
                extract_request_body_swagger2(operation, path_obj, &spec)
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

fn is_json_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    normalized == "application/json" || normalized.ends_with("+json")
}

fn is_concrete_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    normalized != "*/*" && !normalized.is_empty()
}

fn is_text_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    normalized.starts_with("text/")
        || is_json_content_type(&normalized)
        || normalized == "application/xml"
        || normalized.ends_with("+xml")
        || normalized == "application/x-www-form-urlencoded"
        || normalized == "application/yaml"
        || normalized == "application/x-yaml"
        || normalized.ends_with("+yaml")
        || normalized == "application/graphql"
        || normalized == "application/javascript"
        || normalized == "application/ecmascript"
        || normalized == "application/sql"
        || normalized == "application/toml"
        || normalized == "application/ndjson"
        || normalized == "application/x-ndjson"
        || normalized == "application/csv"
        || normalized == "application/tsv"
}

fn is_binary_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    normalized == "application/octet-stream"
        || normalized == "application/zip"
        || normalized == "application/gzip"
        || normalized == "application/pdf"
        || normalized.starts_with("image/")
        || normalized.starts_with("audio/")
        || normalized.starts_with("video/")
        || normalized.starts_with("font/")
        || (normalized.starts_with("application/") && !is_text_content_type(&normalized))
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

fn schema_is_binary(schema: Option<&serde_json::Value>) -> bool {
    schema
        .and_then(|schema| schema.get("format"))
        .and_then(|format| format.as_str())
        == Some("binary")
}

fn schema_contains_binary_field(schema: Option<&serde_json::Value>) -> bool {
    let Some(schema) = schema else {
        return false;
    };

    if schema_is_binary(Some(schema)) {
        return true;
    }

    if let Some(properties) = schema.get("properties").and_then(|value| value.as_object())
        && properties
            .values()
            .any(|property_schema| schema_contains_binary_field(Some(property_schema)))
    {
        return true;
    }

    if let Some(items) = schema.get("items")
        && schema_contains_binary_field(Some(items))
    {
        return true;
    }

    if let Some(additional_properties) = schema.get("additionalProperties")
        && schema_contains_binary_field(Some(additional_properties))
    {
        return true;
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(variants) = schema.get(key).and_then(|value| value.as_array())
            && variants
                .iter()
                .any(|variant_schema| schema_contains_binary_field(Some(variant_schema)))
        {
            return true;
        }
    }

    false
}

fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase()
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
}
