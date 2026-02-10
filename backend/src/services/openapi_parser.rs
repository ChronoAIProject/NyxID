use crate::errors::{AppError, AppResult};

/// A single endpoint parsed from an OpenAPI/Swagger specification.
pub struct ParsedEndpoint {
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
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
            let request_body_schema = if is_openapi3 {
                extract_request_body_openapi3(operation)
            } else {
                extract_request_body_swagger2(operation, path_obj)
            };

            endpoints.push(ParsedEndpoint {
                name,
                description,
                method: method.to_uppercase(),
                path: path.clone(),
                parameters,
                request_body_schema,
            });
        }
    }

    Ok(endpoints)
}

/// Extract or generate a tool-safe name from the operation.
fn extract_name(
    operation: &serde_json::Value,
    method: &str,
    path: &str,
) -> String {
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

/// Extract description from summary or description fields.
fn extract_description(operation: &serde_json::Value) -> Option<String> {
    operation
        .get("summary")
        .and_then(|v| v.as_str())
        .or_else(|| operation.get("description").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
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
fn extract_request_body_openapi3(operation: &serde_json::Value) -> Option<serde_json::Value> {
    operation
        .get("requestBody")
        .and_then(|rb| rb.get("content"))
        .and_then(|content| {
            content
                .get("application/json")
                .or_else(|| content.get("*/*"))
        })
        .and_then(|media| media.get("schema"))
        .cloned()
}

/// Extract body parameter schema for Swagger 2.0.
fn extract_request_body_swagger2(
    operation: &serde_json::Value,
    path_obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
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
    if let Some(params) = operation.get("parameters") {
        if let Some(schema) = find_body_param(params) {
            return Some(schema);
        }
    }

    if let Some(params) = path_obj.get("parameters") {
        if let Some(schema) = find_body_param(&params) {
            return Some(schema);
        }
    }

    None
}
