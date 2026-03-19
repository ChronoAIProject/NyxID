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
}

#[derive(Default)]
struct ParsedRequestBody {
    content_type: Option<String>,
    schema: Option<serde_json::Value>,
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
            let parameters = extract_parameters(operation, path_obj);
            let request_body = if is_openapi3 {
                extract_request_body_openapi3(operation)
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
fn extract_parameters(
    operation: &serde_json::Value,
    path_obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut all_params = Vec::new();

    // Path-level parameters
    if let Some(path_params) = path_obj.get("parameters").and_then(|v| v.as_array()) {
        for p in path_params {
            all_params.push(p.clone());
        }
    }

    // Operation-level parameters (override path-level by name+in)
    if let Some(op_params) = operation.get("parameters").and_then(|v| v.as_array()) {
        for p in op_params {
            all_params.push(p.clone());
        }
    }

    if all_params.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(all_params))
    }
}

/// Extract requestBody schema for OpenAPI 3.x.
fn extract_request_body_openapi3(operation: &serde_json::Value) -> ParsedRequestBody {
    let Some(content) = operation
        .get("requestBody")
        .and_then(|rb| rb.get("content"))
        .and_then(|content| content.as_object())
    else {
        return ParsedRequestBody::default();
    };

    let Some((content_type, media)) = select_openapi3_media(content) else {
        return ParsedRequestBody::default();
    };

    ParsedRequestBody {
        content_type: Some(content_type.to_string()),
        schema: media.get("schema").cloned(),
    }
}

/// Extract body parameter schema for Swagger 2.0.
fn extract_request_body_swagger2(
    operation: &serde_json::Value,
    path_obj: &serde_json::Map<String, serde_json::Value>,
    spec: &serde_json::Value,
) -> ParsedRequestBody {
    let find_body_param = |params: &serde_json::Value| -> Option<serde_json::Value> {
        params.as_array()?.iter().find_map(|p| {
            if p.get("in").and_then(|v| v.as_str()) == Some("body") {
                p.get("schema").cloned()
            } else {
                None
            }
        })
    };

    // Check operation-level first, then path-level
    let schema = if let Some(params) = operation.get("parameters")
        && let Some(schema) = find_body_param(params)
    {
        Some(schema)
    } else if let Some(params) = path_obj.get("parameters")
        && let Some(schema) = find_body_param(params)
    {
        Some(schema)
    } else {
        None
    };

    let content_type = extract_swagger2_consumes(operation, spec, schema.as_ref());

    ParsedRequestBody {
        content_type,
        schema,
    }
}

fn select_openapi3_media(
    content: &serde_json::Map<String, serde_json::Value>,
) -> Option<(&str, &serde_json::Value)> {
    if let Some((content_type, media)) = content.iter().find(|(content_type, media)| {
        is_concrete_content_type(content_type) && is_binary_media(content_type, media)
    }) {
        return Some((content_type.as_str(), media));
    }

    if let Some((content_type, media)) = content
        .iter()
        .find(|(content_type, media)| is_binary_media(content_type, media))
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
) -> Option<String> {
    let prefers_binary = schema_is_binary(body_schema);

    operation
        .get("consumes")
        .and_then(|value| select_swagger2_content_type(value, prefers_binary))
        .or_else(|| {
            spec.get("consumes")
                .and_then(|value| select_swagger2_content_type(value, prefers_binary))
        })
        .or_else(|| prefers_binary.then(|| "application/octet-stream".to_string()))
}

fn select_swagger2_content_type(value: &serde_json::Value, prefers_binary: bool) -> Option<String> {
    let content_types = value.as_array()?;

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

fn is_binary_media(content_type: &str, media: &serde_json::Value) -> bool {
    is_binary_content_type(content_type) || schema_is_binary(media.get("schema"))
}

fn schema_is_binary(schema: Option<&serde_json::Value>) -> bool {
    schema
        .and_then(|schema| schema.get("format"))
        .and_then(|format| format.as_str())
        == Some("binary")
}

fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase()
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
        assert_eq!(body.schema.unwrap()["type"], "object");
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
        assert_eq!(body.schema.unwrap()["type"], "object");
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
}
