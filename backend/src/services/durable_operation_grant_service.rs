use std::collections::{BTreeMap, BTreeSet, HashSet};

use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS};
use crate::models::durable_operation_execution::{
    COLLECTION_NAME as EXECUTIONS, DurableExecutionStatus, DurableOperationExecution,
};
use crate::models::durable_operation_grant::{
    COLLECTION_NAME as GRANTS, DurableBodyConstraint, DurableOperationConstraints,
    DurableOperationGrant, DurableOperationPlan, DurableOperationSelection,
    DurableParameterConstraint, DurableReplayPolicy, DurableValueConstraint,
};
use crate::models::service_endpoint::{EndpointRisk, ServiceEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::mcp_service::{NodeScope, ServiceScope};
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::{api_key_scope_service, key_service, mcp_service};

pub const DURABLE_GRANT_CONTRACT_VERSION: &str = "durable-operation-grant-v1";
pub const DURABLE_GRANT_HEADER: &str = "x-nyxid-durable-grant-id";
pub const OPERATION_ID_HEADER: &str = "x-nyxid-operation-id";
const MAX_OPERATION_ID_LEN: usize = 200;
const MAX_CAS_ATTEMPTS: usize = 16;

#[derive(Clone, Debug)]
pub struct DurableExecutionReservation {
    pub execution_id: String,
    pub grant_id: String,
    pub operation_id: String,
    pub endpoint_id: String,
    pub contract_digest: String,
    pub replay_policy: DurableReplayPolicy,
    pub client_audit_binding:
        Option<crate::models::durable_operation_grant::DurableClientAuditBinding>,
}

pub struct ProvisionedScheduledKey {
    pub key: key_service::CreatedApiKey,
    pub grants: Vec<DurableOperationGrant>,
}

fn validation(message: impl Into<String>) -> AppError {
    AppError::ValidationError(message.into())
}

pub fn parse_contract_datetime(value: &str, field: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(|_| validation(format!("{field} must be an RFC 3339 timestamp")))
}

fn normalize_path(value: &str) -> AppResult<String> {
    let path = value.split_once('?').map_or(value, |(path, _)| path);
    if path.contains('*') || path.contains("..") || path.contains("//") {
        return Err(validation(
            "durable operation paths cannot contain wildcards, traversal, or empty segments",
        ));
    }
    let normalized = format!("/{}", path.trim_matches('/'));
    if normalized.len() > 2048 {
        return Err(validation(
            "durable operation path must not exceed 2048 characters",
        ));
    }
    Ok(normalized)
}

fn path_variable_names(path: &str) -> AppResult<Vec<String>> {
    let mut names = Vec::new();
    for segment in path.trim_matches('/').split('/') {
        let has_brace = segment.contains('{') || segment.contains('}');
        if !has_brace {
            continue;
        }
        if !(segment.starts_with('{') && segment.ends_with('}') && segment.len() > 2) {
            return Err(validation(
                "durable operation path variables must occupy a complete path segment",
            ));
        }
        let name = &segment[1..segment.len() - 1];
        if name.contains('{') || name.contains('}') || !names.iter().all(|entry| entry != name) {
            return Err(validation(
                "durable operation path variables must be unique and well formed",
            ));
        }
        names.push(name.to_string());
    }
    Ok(names)
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}

fn hash_canonical(value: &Value) -> String {
    let bytes =
        serde_json::to_vec(&canonicalize_json(value)).expect("JSON values always serialize");
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

pub fn endpoint_contract_digest(endpoint: &ServiceEndpoint) -> AppResult<String> {
    let normalized_path = normalize_path(&endpoint.path)?;
    Ok(hash_canonical(&serde_json::json!({
        "authority": "nyxid",
        "contract_version": DURABLE_GRANT_CONTRACT_VERSION,
        "endpoint_id": endpoint.id,
        "service_id": endpoint.service_id,
        "method": endpoint.method.to_uppercase(),
        "path": normalized_path,
        "parameters": endpoint.parameters,
        "request_body_schema": endpoint.request_body_schema,
        "request_content_type": endpoint.request_content_type,
        "request_body_required": endpoint.effective_request_body_required(),
        "risk": endpoint.risk,
        "supports_idempotency_key": endpoint.supports_idempotency_key,
    })))
}

async fn load_active_published_endpoint(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    owner_user_id: &str,
    user_service_id: &str,
    allowed_node_ids: &[String],
    endpoint_id: &str,
) -> AppResult<Option<ServiceEndpoint>> {
    let catalog_backed = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {
            "_id": user_service_id,
            "user_id": owner_user_id,
            "is_active": true,
            "service_type": "http",
            "catalog_service_id": { "$type": "string", "$ne": "" },
        })
        .await?
        .is_some();
    if !catalog_backed {
        return Ok(None);
    }

    let service_ids = [user_service_id.to_string()];
    let catalog = mcp_service::load_operation_catalog(
        db,
        node_ws_manager,
        owner_user_id,
        NodeScope::Allowed(allowed_node_ids),
        ServiceScope::Allowed(&service_ids),
    )
    .await?;
    let Some(service) = catalog
        .services
        .into_iter()
        .find(|service| service.service_id == user_service_id && !service.is_generic_proxy)
    else {
        return Ok(None);
    };
    let Some(endpoint) = service
        .endpoints
        .into_iter()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
    else {
        return Ok(None);
    };
    let Some(metadata) = service.durable_endpoint_metadata.get(endpoint_id).copied() else {
        return Ok(None);
    };
    let now = Utc::now();
    Ok(Some(ServiceEndpoint {
        id: endpoint.endpoint_id,
        service_id: user_service_id.to_string(),
        name: endpoint.name,
        description: endpoint.description,
        method: endpoint.method,
        path: endpoint.path,
        parameters: endpoint.parameters,
        request_body_schema: endpoint.request_body_schema,
        request_content_type: endpoint.request_content_type,
        request_body_required: endpoint.request_body_required,
        response_description: endpoint.response_description,
        response: endpoint.response,
        risk: metadata.risk,
        supports_idempotency_key: metadata.supports_idempotency_key,
        is_active: true,
        created_at: now,
        updated_at: now,
    }))
}

fn validate_rule(rule: &DurableValueConstraint, scalar_only: bool) -> AppResult<()> {
    let values: Vec<&Value> = match rule {
        DurableValueConstraint::Exact { value } => vec![value],
        DurableValueConstraint::OneOf { values } => {
            if values.is_empty() || values.len() > 100 {
                return Err(validation(
                    "one_of constraints must contain between 1 and 100 values",
                ));
            }
            let unique: HashSet<String> = values
                .iter()
                .map(|value| serde_json::to_string(&canonicalize_json(value)).unwrap_or_default())
                .collect();
            if unique.len() != values.len() {
                return Err(validation("one_of constraints must not contain duplicates"));
            }
            values.iter().collect()
        }
    };
    if scalar_only
        && values.iter().any(|value| {
            !matches!(
                value,
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
            )
        })
    {
        return Err(validation(
            "path, query, and header constraints must use scalar values",
        ));
    }
    Ok(())
}

fn validate_client_audit_binding(
    binding: &crate::models::durable_operation_grant::DurableClientAuditBinding,
) -> AppResult<()> {
    for (name, value) in [
        ("platform", binding.platform.as_deref()),
        ("schedule_id", binding.schedule_id.as_deref()),
        ("workflow_revision", binding.workflow_revision.as_deref()),
        ("call_site", binding.call_site.as_deref()),
    ] {
        if let Some(value) = value
            && (value.is_empty() || value.len() > 256 || value.chars().any(char::is_control))
        {
            return Err(validation(format!(
                "client_audit_binding.{name} must contain 1 to 256 bytes without control characters"
            )));
        }
    }
    Ok(())
}

fn valid_json_pointer(pointer: &str) -> bool {
    if pointer.is_empty() {
        return true;
    }
    if !pointer.starts_with('/') {
        return false;
    }
    let bytes = pointer.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'~' {
            if index + 1 >= bytes.len() || !matches!(bytes[index + 1], b'0' | b'1') {
                return false;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    true
}

#[derive(Clone)]
struct ParameterDefinition {
    location: String,
    name: String,
    required: bool,
}

fn parameter_definitions(endpoint: &ServiceEndpoint) -> Vec<ParameterDefinition> {
    endpoint
        .parameters
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|parameter| {
            Some(ParameterDefinition {
                location: parameter.get("in")?.as_str()?.to_ascii_lowercase(),
                name: parameter.get("name")?.as_str()?.to_string(),
                required: parameter
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn normalize_and_validate_constraints(
    endpoint: &ServiceEndpoint,
    constraints: &DurableOperationConstraints,
) -> AppResult<DurableOperationConstraints> {
    let path = normalize_path(&endpoint.path)?;
    let variables = path_variable_names(&path)?;
    let definitions = parameter_definitions(endpoint);
    let declared_path: BTreeSet<String> = definitions
        .iter()
        .filter(|definition| definition.location == "path")
        .map(|definition| definition.name.clone())
        .chain(variables.iter().cloned())
        .collect();
    let declared_query: BTreeSet<String> = definitions
        .iter()
        .filter(|definition| definition.location == "query")
        .map(|definition| definition.name.clone())
        .collect();
    let declared_headers: BTreeSet<String> = definitions
        .iter()
        .filter(|definition| definition.location == "header")
        .map(|definition| definition.name.to_ascii_lowercase())
        .collect();

    if variables.iter().any(|name| {
        !constraints
            .path
            .get(name)
            .is_some_and(|constraint| constraint.required)
    }) {
        return Err(validation(
            "every path variable requires a bounded required constraint",
        ));
    }
    if constraints
        .path
        .keys()
        .any(|name| !declared_path.contains(name))
    {
        return Err(validation(
            "path constraints must name declared path variables",
        ));
    }
    if constraints
        .query
        .keys()
        .any(|name| !declared_query.contains(name))
    {
        return Err(validation(
            "query constraints must name declared query parameters",
        ));
    }

    let mut normalized_headers = BTreeMap::new();
    for (name, constraint) in &constraints.headers {
        let normalized = name.to_ascii_lowercase();
        if normalized == "authorization"
            || normalized == "cookie"
            || normalized == "idempotency-key"
            || normalized.starts_with("x-nyxid-")
            || !declared_headers.contains(&normalized)
        {
            return Err(validation(
                "header constraints must name declared non-system headers",
            ));
        }
        if normalized_headers
            .insert(normalized, constraint.clone())
            .is_some()
        {
            return Err(validation(
                "header constraints must be unique ignoring case",
            ));
        }
    }

    for definition in &definitions {
        if !definition.required {
            continue;
        }
        let present = match definition.location.as_str() {
            "path" => constraints.path.get(&definition.name),
            "query" => constraints.query.get(&definition.name),
            "header" => normalized_headers.get(&definition.name.to_ascii_lowercase()),
            _ => continue,
        };
        if !present.is_some_and(|constraint| constraint.required) {
            return Err(validation(format!(
                "required {} parameter '{}' needs a required constraint",
                definition.location, definition.name
            )));
        }
    }

    for constraint in constraints
        .path
        .values()
        .chain(constraints.query.values())
        .chain(normalized_headers.values())
    {
        validate_rule(&constraint.rule, true)?;
    }

    if let Some(body) = &constraints.body {
        if body.allow_additional_fields {
            return Err(validation(
                "allow_additional_fields is unsupported for durable grants",
            ));
        }
        if body.fields.is_empty() {
            return Err(validation(
                "body constraints must contain at least one field",
            ));
        }
        if body.fields.contains_key("") && body.fields.len() != 1 {
            return Err(validation(
                "the root body constraint cannot be combined with child JSON Pointers",
            ));
        }
        let pointers: Vec<&str> = body.fields.keys().map(String::as_str).collect();
        if pointers.iter().enumerate().any(|(index, pointer)| {
            pointers.iter().skip(index + 1).any(|other| {
                other.starts_with(&format!("{pointer}/"))
                    || pointer.starts_with(&format!("{other}/"))
            })
        }) {
            return Err(validation("body constraint JSON Pointers must not overlap"));
        }
        for (pointer, constraint) in &body.fields {
            if !valid_json_pointer(pointer) {
                return Err(validation(
                    "body constraint keys must be valid JSON Pointers or the empty root pointer",
                ));
            }
            validate_rule(&constraint.rule, false)?;
        }
    }
    if endpoint.effective_request_body_required() && constraints.body.is_none() {
        return Err(validation(
            "a required request body needs bounded body constraints",
        ));
    }

    Ok(DurableOperationConstraints {
        path: constraints.path.clone(),
        query: constraints.query.clone(),
        headers: normalized_headers,
        body: constraints.body.clone(),
    })
}

pub async fn build_operation_plans(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    owner_user_id: &str,
    allowed_node_ids: &[String],
    selections: &[DurableOperationSelection],
    key_expires_at: Option<DateTime<Utc>>,
) -> AppResult<Vec<DurableOperationPlan>> {
    if selections.is_empty() {
        return Ok(Vec::new());
    }
    let key_expires_at = key_expires_at.ok_or_else(|| {
        validation("scheduled_invocation keys require a finite expires_at timestamp")
    })?;
    let now = Utc::now();
    let mut seen = HashSet::new();
    let mut plans = Vec::with_capacity(selections.len());

    for selection in selections {
        if !seen.insert((
            selection.user_service_id.as_str(),
            selection.endpoint_id.as_str(),
        )) {
            return Err(validation(
                "selected_operations must not contain duplicate service/endpoint pairs",
            ));
        }
        if selection.total_limit <= 0 {
            return Err(validation("total_limit must be greater than zero"));
        }
        if let Some(window) = &selection.window
            && (window.duration_seconds <= 0
                || Duration::try_seconds(window.duration_seconds).is_none()
                || window.max_operations <= 0)
        {
            return Err(validation(
                "window duration_seconds and max_operations must be greater than zero",
            ));
        }
        if let Some(binding) = selection.client_audit_binding.as_ref() {
            validate_client_audit_binding(binding)?;
        }

        let endpoint = load_active_published_endpoint(
            db,
            node_ws_manager,
            owner_user_id,
            &selection.user_service_id,
            allowed_node_ids,
            &selection.endpoint_id,
        )
        .await?
        .ok_or_else(|| {
            AppError::ApiKeyScopePlanNotFound(format!(
                "active published endpoint '{}' not found for UserService '{}'",
                selection.endpoint_id, selection.user_service_id
            ))
        })?;

        let method = endpoint.method.to_uppercase();
        if !matches!(method.as_str(), "POST" | "PUT" | "PATCH")
            || endpoint.risk != Some(EndpointRisk::Write)
        {
            return Err(validation(
                "durable grants require a POST, PUT, or PATCH endpoint explicitly classified as write",
            ));
        }
        if selection.replay_policy == DurableReplayPolicy::DownstreamIdempotencyKey
            && !endpoint.supports_idempotency_key
        {
            return Err(validation(
                "downstream_idempotency_key requires explicit endpoint contract support",
            ));
        }

        let valid_from = parse_contract_datetime(&selection.valid_from, "valid_from")?;
        let expires_at = parse_contract_datetime(&selection.expires_at, "expires_at")?;
        if expires_at <= valid_from || expires_at <= now || expires_at > key_expires_at {
            return Err(validation(
                "grant expires_at must follow valid_from, be in the future, and not exceed key expiry",
            ));
        }

        let normalized_path_template = normalize_path(&endpoint.path)?;
        let constraints = normalize_and_validate_constraints(&endpoint, &selection.constraints)?;
        plans.push(DurableOperationPlan {
            user_service_id: selection.user_service_id.clone(),
            endpoint_id: endpoint.id.clone(),
            method,
            normalized_path_template,
            contract_digest: endpoint_contract_digest(&endpoint)?,
            constraints,
            valid_from: valid_from.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
            total_limit: selection.total_limit,
            window: selection.window.clone(),
            replay_policy: selection.replay_policy,
            client_audit_binding: selection.client_audit_binding.clone(),
        });
    }
    plans.sort_by(|left, right| {
        (&left.user_service_id, &left.endpoint_id)
            .cmp(&(&right.user_service_id, &right.endpoint_id))
    });
    Ok(plans)
}

pub fn plan_digest_component(plans: &[DurableOperationPlan]) -> Value {
    canonicalize_json(&serde_json::to_value(plans).expect("operation plans serialize"))
}

fn constraint_matches(rule: &DurableValueConstraint, value: &Value) -> bool {
    match rule {
        DurableValueConstraint::Exact { value: expected } => expected == value,
        DurableValueConstraint::OneOf { values } => values.iter().any(|expected| expected == value),
    }
}

fn validate_parameter_constraints(
    constraints: &BTreeMap<String, DurableParameterConstraint>,
    actual: &BTreeMap<String, Value>,
    class: &str,
) -> AppResult<()> {
    for (name, value) in actual {
        let constraint = constraints.get(name).ok_or_else(|| {
            AppError::DurableGrantMismatch(format!("unconstrained {class} parameter '{name}'"))
        })?;
        if !constraint_matches(&constraint.rule, value) {
            return Err(AppError::DurableGrantMismatch(format!(
                "{class} parameter '{name}' violates its grant constraint"
            )));
        }
    }
    for (name, constraint) in constraints {
        if constraint.required && !actual.contains_key(name) {
            return Err(AppError::DurableGrantMismatch(format!(
                "required {class} parameter '{name}' is missing"
            )));
        }
    }
    Ok(())
}

fn resolve_path_arguments(template: &str, actual_path: &str) -> AppResult<BTreeMap<String, Value>> {
    let actual = normalize_path(actual_path)
        .map_err(|_| AppError::DurableGrantMismatch("request path is not canonical".to_string()))?;
    let template_segments: Vec<&str> = template.trim_matches('/').split('/').collect();
    let actual_segments: Vec<&str> = actual.trim_matches('/').split('/').collect();
    if template_segments.len() != actual_segments.len() {
        return Err(AppError::DurableGrantMismatch(
            "request path does not match the granted endpoint template".to_string(),
        ));
    }
    let mut arguments = BTreeMap::new();
    for (template_segment, actual_segment) in template_segments.iter().zip(actual_segments) {
        if template_segment.starts_with('{') && template_segment.ends_with('}') {
            let name = &template_segment[1..template_segment.len() - 1];
            let decoded = urlencoding::decode(actual_segment).map_err(|_| {
                AppError::DurableGrantMismatch("path argument is not valid UTF-8".to_string())
            })?;
            arguments.insert(name.to_string(), Value::String(decoded.into_owned()));
        } else if *template_segment != actual_segment {
            return Err(AppError::DurableGrantMismatch(
                "request path does not match the granted endpoint template".to_string(),
            ));
        }
    }
    Ok(arguments)
}

fn parse_query(query: Option<&str>) -> AppResult<BTreeMap<String, Value>> {
    let mut parsed = BTreeMap::new();
    for (name, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        if parsed
            .insert(name.into_owned(), Value::String(value.into_owned()))
            .is_some()
        {
            return Err(AppError::DurableGrantMismatch(
                "duplicate query parameters are unsupported for durable operations".to_string(),
            ));
        }
    }
    Ok(parsed)
}

fn constrained_headers(
    endpoint: &ServiceEndpoint,
    headers: &HeaderMap,
) -> AppResult<BTreeMap<String, Value>> {
    let mut actual = BTreeMap::new();
    let declared_headers: BTreeSet<String> = parameter_definitions(endpoint)
        .into_iter()
        .filter(|definition| definition.location == "header")
        .map(|definition| definition.name.to_ascii_lowercase())
        .collect();

    // These caller-controlled headers are forwarded by the proxy and can
    // change operation selection, permissions, billing, or write
    // preconditions. They must therefore be part of the published endpoint
    // contract before a durable grant may constrain and authorize them.
    for name in headers.keys().map(|name| name.as_str()) {
        let affects_operation = matches!(
            name,
            "range"
                | "if-range"
                | "if-none-match"
                | "if-modified-since"
                | "http-referer"
                | "x-title"
        ) || ["x-openclaw-", "x-amz-", "x-goog-", "x-openrouter-"]
            .iter()
            .any(|prefix| name.starts_with(prefix));
        if affects_operation && !declared_headers.contains(name) {
            return Err(AppError::DurableGrantMismatch(format!(
                "operation-affecting header '{name}' is not declared by the endpoint contract"
            )));
        }
    }

    for name in declared_headers {
        let mut values = headers.get_all(&name).iter();
        if let Some(value) = values.next() {
            if values.next().is_some() {
                return Err(AppError::DurableGrantMismatch(format!(
                    "header '{name}' must be supplied at most once"
                )));
            }
            actual.insert(
                name.clone(),
                Value::String(
                    value
                        .to_str()
                        .map_err(|_| {
                            AppError::DurableGrantMismatch(format!(
                                "header '{name}' is not valid text"
                            ))
                        })?
                        .to_string(),
                ),
            );
        }
    }
    Ok(actual)
}

fn validate_request_content_type(
    endpoint: &ServiceEndpoint,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<()> {
    if body.is_empty() {
        return Ok(());
    }
    let actual = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or_default().trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::DurableGrantMismatch(
                "a durable JSON request body requires Content-Type".to_string(),
            )
        })?;
    if !crate::services::content_type::is_json_content_type(actual) {
        return Err(AppError::DurableGrantMismatch(
            "durable operation request Content-Type must be JSON".to_string(),
        ));
    }
    if let Some(expected) = endpoint.request_content_type.as_deref() {
        let expected = expected.split(';').next().unwrap_or_default().trim();
        if !expected.is_empty() && !actual.eq_ignore_ascii_case(expected) {
            return Err(AppError::DurableGrantMismatch(
                "request Content-Type does not match the endpoint contract".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_body_constraint(
    endpoint: &ServiceEndpoint,
    constraint: Option<&DurableBodyConstraint>,
    body: &[u8],
) -> AppResult<Option<Value>> {
    if body.is_empty() {
        if endpoint.effective_request_body_required() {
            return Err(AppError::DurableGrantMismatch(
                "required request body is missing".to_string(),
            ));
        }
        if let Some(constraint) = constraint
            && constraint.fields.values().any(|field| field.required)
        {
            return Err(AppError::DurableGrantMismatch(
                "required body constraint is missing".to_string(),
            ));
        }
        return Ok(None);
    }

    let content_type = endpoint.request_content_type.as_deref().unwrap_or_default();
    if !content_type.is_empty()
        && !crate::services::content_type::is_json_content_type(content_type)
    {
        return Err(AppError::DurableGrantMismatch(
            "raw or binary request bodies are unsupported for durable operations".to_string(),
        ));
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| {
        AppError::DurableGrantMismatch(
            "durable operation request body must be valid JSON".to_string(),
        )
    })?;
    let constraint = constraint.ok_or_else(|| {
        AppError::DurableGrantMismatch("request body is not authorized by the grant".to_string())
    })?;

    if let Some(root) = constraint.fields.get("") {
        if !constraint_matches(&root.rule, &value) {
            return Err(AppError::DurableGrantMismatch(
                "request body violates its root constraint".to_string(),
            ));
        }
        return Ok(Some(value));
    }

    let mut leaves = Vec::new();
    fn collect_leaves<'a>(
        value: &'a Value,
        pointer: String,
        output: &mut Vec<(String, &'a Value)>,
    ) {
        match value {
            Value::Object(object) if !object.is_empty() => {
                for (name, child) in object {
                    let escaped = name.replace('~', "~0").replace('/', "~1");
                    collect_leaves(child, format!("{pointer}/{escaped}"), output);
                }
            }
            Value::Array(values) if !values.is_empty() => {
                for (index, child) in values.iter().enumerate() {
                    collect_leaves(child, format!("{pointer}/{index}"), output);
                }
            }
            _ => output.push((pointer, value)),
        }
    }
    collect_leaves(&value, String::new(), &mut leaves);
    for (pointer, leaf) in leaves {
        let field = constraint.fields.get(&pointer).ok_or_else(|| {
            AppError::DurableGrantMismatch(format!(
                "request body field '{pointer}' is unconstrained"
            ))
        })?;
        if !constraint_matches(&field.rule, leaf) {
            return Err(AppError::DurableGrantMismatch(format!(
                "request body field '{pointer}' violates its grant constraint"
            )));
        }
    }
    for (pointer, field) in &constraint.fields {
        if field.required && value.pointer(pointer).is_none() {
            return Err(AppError::DurableGrantMismatch(format!(
                "required request body field '{pointer}' is missing"
            )));
        }
    }
    Ok(Some(value))
}

fn valid_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OPERATION_ID_LEN
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character))
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

async fn reject_execution(db: &mongodb::Database, execution_id: &str, detail: &str) {
    let now = Utc::now();
    let _ = db
        .collection::<DurableOperationExecution>(EXECUTIONS)
        .update_one(
            doc! { "_id": execution_id, "status": "reserved" },
            doc! { "$set": {
                "status": "rejected",
                "terminal_detail": detail,
                "terminal_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await;
}

async fn claim_quota(db: &mongodb::Database, grant_id: &str, now: DateTime<Utc>) -> AppResult<()> {
    let grants = db.collection::<DurableOperationGrant>(GRANTS);
    for _ in 0..MAX_CAS_ATTEMPTS {
        let grant = grants
            .find_one(doc! { "_id": grant_id })
            .await?
            .ok_or_else(|| AppError::DurableGrantMissing("grant not found".to_string()))?;
        if grant.revoked_at.is_some() {
            return Err(AppError::DurableGrantRevoked);
        }
        if now < grant.valid_from {
            return Err(AppError::DurableGrantMismatch(
                "grant is not valid yet".to_string(),
            ));
        }
        if now >= grant.expires_at {
            return Err(AppError::DurableGrantExpired);
        }
        if grant.total_used >= grant.total_limit {
            return Err(AppError::DurableGrantQuotaExhausted);
        }

        let (window_started_at, window_used) = if let Some(window) = &grant.window {
            let duration = Duration::try_seconds(window.duration_seconds)
                .ok_or(AppError::DurableGrantContractDrift)?;
            let active_start = grant.window_started_at.filter(|start| {
                start
                    .checked_add_signed(duration)
                    .is_some_and(|end| now < end)
            });
            let start = active_start.unwrap_or(now);
            let used = if active_start.is_some() {
                grant.window_used
            } else {
                0
            };
            if used >= window.max_operations {
                return Err(AppError::DurableGrantQuotaExhausted);
            }
            (Some(start), used + 1)
        } else {
            (grant.window_started_at, grant.window_used)
        };

        let result = grants
            .update_one(
                doc! {
                    "_id": grant_id,
                    "state_version": grant.state_version,
                    "revoked_at": bson::Bson::Null,
                },
                doc! { "$set": {
                    "window_started_at": window_started_at.map(bson::DateTime::from_chrono),
                    "window_used": window_used,
                    "updated_at": bson::DateTime::from_chrono(now),
                }, "$inc": {
                    "total_used": 1_i64,
                    "state_version": 1_i64,
                }},
            )
            .await?;
        if result.modified_count == 1 {
            return Ok(());
        }
    }
    Err(AppError::DurableGrantQuotaExhausted)
}

#[allow(clippy::too_many_arguments)]
pub async fn authorize_and_reserve(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    owner_user_id: &str,
    api_key_id: &str,
    user_service_id: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: &[u8],
    grant_id: &str,
    operation_id: &str,
    is_websocket: bool,
) -> AppResult<DurableExecutionReservation> {
    if grant_id.is_empty() {
        return Err(AppError::DurableGrantMissing(
            "X-NyxID-Durable-Grant-Id is required".to_string(),
        ));
    }
    if !valid_operation_id(operation_id) {
        return Err(AppError::DurableGrantMissing(
            "X-NyxID-Operation-Id is required and must be a bounded ASCII identifier".to_string(),
        ));
    }
    if is_websocket {
        return Err(AppError::DurableGrantMismatch(
            "WebSocket operations cannot use durable grants".to_string(),
        ));
    }

    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": api_key_id, "user_id": owner_user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::DurableGrantMismatch("API key is inactive".to_string()))?;
    if key.purpose != ApiKeyPurpose::ScheduledInvocation || !key.scheduled_write_enabled {
        return Err(AppError::DurableGrantMismatch(
            "API key is not an active scheduled_invocation credential".to_string(),
        ));
    }
    if key.allow_all_services
        || key.allow_all_nodes
        || !key
            .allowed_service_ids
            .iter()
            .any(|id| id == user_service_id)
    {
        return Err(AppError::DurableGrantMismatch(
            "scheduled_invocation key scope is not exact".to_string(),
        ));
    }

    let grant = db
        .collection::<DurableOperationGrant>(GRANTS)
        .find_one(doc! { "_id": grant_id })
        .await?
        .ok_or_else(|| AppError::DurableGrantMissing("grant not found".to_string()))?;
    if grant.user_id != owner_user_id
        || grant.api_key_id != api_key_id
        || grant.user_service_id != user_service_id
    {
        return Err(AppError::DurableGrantMismatch(
            "grant does not match the exact owner, API key, or UserService".to_string(),
        ));
    }
    if grant.revoked_at.is_some() {
        return Err(AppError::DurableGrantRevoked);
    }
    let now = Utc::now();
    if now < grant.valid_from {
        return Err(AppError::DurableGrantMismatch(
            "grant is not valid yet".to_string(),
        ));
    }
    if now >= grant.expires_at {
        return Err(AppError::DurableGrantExpired);
    }

    let endpoint = load_active_published_endpoint(
        db,
        node_ws_manager,
        owner_user_id,
        user_service_id,
        &key.allowed_node_ids,
        &grant.endpoint_id,
    )
    .await?
    .ok_or(AppError::DurableGrantContractDrift)?;
    if endpoint.risk != Some(EndpointRisk::Write)
        || endpoint_contract_digest(&endpoint)? != grant.contract_digest
    {
        return Err(AppError::DurableGrantContractDrift);
    }
    if method.to_uppercase() != grant.method {
        return Err(AppError::DurableGrantMismatch(
            "request method does not match the grant".to_string(),
        ));
    }

    let path_arguments = resolve_path_arguments(&grant.normalized_path_template, path)?;
    validate_parameter_constraints(&grant.constraints.path, &path_arguments, "path")?;
    let query_arguments = parse_query(query)?;
    validate_parameter_constraints(&grant.constraints.query, &query_arguments, "query")?;
    let header_arguments = constrained_headers(&endpoint, headers)?;
    validate_parameter_constraints(&grant.constraints.headers, &header_arguments, "header")?;
    validate_request_content_type(&endpoint, headers, body)?;
    let body_value = validate_body_constraint(&endpoint, grant.constraints.body.as_ref(), body)?;

    let request_digest = hash_canonical(&serde_json::json!({
        "method": method.to_uppercase(),
        "path": normalize_path(path).map_err(|_| AppError::DurableGrantMismatch("request path is not canonical".to_string()))?,
        "path_arguments": path_arguments,
        "query": query_arguments,
        "headers": header_arguments,
        "body": body_value,
    }));
    let execution_id = Uuid::new_v4().to_string();
    let execution = DurableOperationExecution {
        id: execution_id.clone(),
        grant_id: grant.id.clone(),
        operation_id: operation_id.to_string(),
        api_key_id: api_key_id.to_string(),
        user_id: owner_user_id.to_string(),
        user_service_id: user_service_id.to_string(),
        endpoint_id: grant.endpoint_id.clone(),
        contract_digest: grant.contract_digest.clone(),
        request_digest: request_digest.clone(),
        status: DurableExecutionStatus::Reserved,
        downstream_attempts: 0,
        node_id: None,
        response_status: None,
        terminal_detail: None,
        dispatched_at: None,
        terminal_at: None,
        created_at: now,
        updated_at: now,
    };
    if let Err(error) = db
        .collection::<DurableOperationExecution>(EXECUTIONS)
        .insert_one(&execution)
        .await
    {
        if !is_duplicate_key_error(&error) {
            return Err(error.into());
        }
        let existing = db
            .collection::<DurableOperationExecution>(EXECUTIONS)
            .find_one(doc! { "grant_id": grant_id, "operation_id": operation_id })
            .await?;
        return Err(match existing {
            Some(existing) if existing.request_digest != request_digest => {
                AppError::DurableOperationConflict
            }
            Some(existing)
                if matches!(
                    existing.status,
                    DurableExecutionStatus::Dispatched | DurableExecutionStatus::OutcomeUncertain
                ) =>
            {
                AppError::DurableOperationOutcomeUncertain
            }
            _ => AppError::DurableOperationDuplicate,
        });
    }

    if let Err(error) = claim_quota(db, &grant.id, now).await {
        reject_execution(db, &execution_id, &error.to_string()).await;
        return Err(error);
    }

    Ok(DurableExecutionReservation {
        execution_id,
        grant_id: grant.id,
        operation_id: operation_id.to_string(),
        endpoint_id: grant.endpoint_id,
        contract_digest: grant.contract_digest,
        replay_policy: grant.replay_policy,
        client_audit_binding: grant.client_audit_binding,
    })
}

pub async fn mark_dispatched(
    db: &mongodb::Database,
    reservation: &DurableExecutionReservation,
    node_id: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now();
    let result = db
        .collection::<DurableOperationExecution>(EXECUTIONS)
        .update_one(
            doc! { "_id": &reservation.execution_id, "status": "reserved" },
            doc! { "$set": {
                "status": "dispatched",
                "downstream_attempts": 1_i64,
                "node_id": node_id,
                "dispatched_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;
    if result.modified_count != 1 {
        return Err(AppError::DurableOperationOutcomeUncertain);
    }
    Ok(())
}

pub async fn mark_pre_dispatch_rejected(
    db: &mongodb::Database,
    reservation: &DurableExecutionReservation,
    detail: &str,
) {
    reject_execution(db, &reservation.execution_id, detail).await;
}

pub async fn mark_terminal(
    db: &mongodb::Database,
    reservation: &DurableExecutionReservation,
    status: DurableExecutionStatus,
    response_status: Option<u16>,
    detail: &str,
) {
    let now = Utc::now();
    let status_bson =
        bson::to_bson(&status).unwrap_or(bson::Bson::String("outcome_uncertain".to_string()));
    let _ = db
        .collection::<DurableOperationExecution>(EXECUTIONS)
        .update_one(
            doc! { "_id": &reservation.execution_id, "status": "dispatched" },
            doc! { "$set": {
                "status": status_bson,
                "response_status": response_status.map(i64::from),
                "terminal_detail": detail,
                "terminal_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await;
}

pub async fn list_grants(
    db: &mongodb::Database,
    owner_user_id: &str,
    api_key_id: &str,
    include_revoked: bool,
) -> AppResult<Vec<DurableOperationGrant>> {
    let mut filter = doc! { "user_id": owner_user_id, "api_key_id": api_key_id };
    if !include_revoked {
        filter.insert("revoked_at", bson::Bson::Null);
    }
    Ok(db
        .collection::<DurableOperationGrant>(GRANTS)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?)
}

pub async fn revoke_grant(
    db: &mongodb::Database,
    owner_user_id: &str,
    api_key_id: &str,
    grant_id: &str,
    actor_user_id: &str,
) -> AppResult<DurableOperationGrant> {
    let now = Utc::now();
    let result = db
        .collection::<DurableOperationGrant>(GRANTS)
        .find_one_and_update(
            doc! {
                "_id": grant_id,
                "user_id": owner_user_id,
                "api_key_id": api_key_id,
                "revoked_at": bson::Bson::Null,
            },
            doc! { "$set": {
                "revoked_at": bson::DateTime::from_chrono(now),
                "revoked_by": actor_user_id,
                "updated_at": bson::DateTime::from_chrono(now),
            }, "$inc": { "state_version": 1_i64 } },
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::DurableGrantMissing("active grant not found".to_string()))?;
    Ok(result)
}

pub fn grant_from_plan(
    owner_user_id: &str,
    api_key_id: &str,
    actor_user_id: &str,
    plan: &DurableOperationPlan,
    reauthorized_from: Option<&str>,
) -> AppResult<DurableOperationGrant> {
    let now = Utc::now();
    Ok(DurableOperationGrant {
        id: Uuid::new_v4().to_string(),
        user_id: owner_user_id.to_string(),
        api_key_id: api_key_id.to_string(),
        user_service_id: plan.user_service_id.clone(),
        endpoint_id: plan.endpoint_id.clone(),
        method: plan.method.clone(),
        normalized_path_template: plan.normalized_path_template.clone(),
        contract_digest: plan.contract_digest.clone(),
        constraints: plan.constraints.clone(),
        valid_from: parse_contract_datetime(&plan.valid_from, "valid_from")?,
        expires_at: parse_contract_datetime(&plan.expires_at, "expires_at")?,
        total_limit: plan.total_limit,
        total_used: 0,
        window: plan.window.clone(),
        window_started_at: None,
        window_used: 0,
        replay_policy: plan.replay_policy,
        client_audit_binding: plan.client_audit_binding.clone(),
        revoked_at: None,
        revoked_by: None,
        state_version: 1,
        created_by: actor_user_id.to_string(),
        reauthorized_from: reauthorized_from.map(str::to_string),
        created_at: now,
        updated_at: now,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn provision_scheduled_key(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    actor_user_id: &str,
    owner_user_id: &str,
    name: &str,
    expires_at: DateTime<Utc>,
    description: Option<&str>,
    allowed_service_ids: &[String],
    allowed_node_ids: &[String],
    rate_limit_per_second: Option<u32>,
    rate_limit_burst: Option<u32>,
    platform: Option<&str>,
    selections: &[DurableOperationSelection],
    expected_digest: &str,
) -> AppResult<ProvisionedScheduledKey> {
    let plan = api_key_scope_service::verify_durable_scope_plan_precondition(
        db,
        node_ws_manager,
        actor_user_id,
        owner_user_id,
        allowed_service_ids,
        allowed_node_ids,
        selections,
        expires_at,
        expected_digest,
    )
    .await?;

    let key = key_service::create_api_key_with_security_class(
        db,
        owner_user_id,
        Some(actor_user_id),
        name,
        "proxy",
        Some(expires_at),
        description,
        Some(&plan.allowed_service_ids),
        Some(&plan.allowed_node_ids),
        Some(false),
        Some(false),
        rate_limit_per_second,
        rate_limit_burst,
        platform,
        None,
        None,
        ApiKeyPurpose::ScheduledInvocation,
        false,
    )
    .await?;

    let grants: Vec<DurableOperationGrant> = plan
        .durable_operations
        .iter()
        .map(|operation| grant_from_plan(owner_user_id, &key.id, actor_user_id, operation, None))
        .collect::<AppResult<_>>()?;

    let provision_result: AppResult<()> = async {
        if grants.is_empty() {
            return Err(validation(
                "scheduled_invocation provisioning requires at least one durable operation",
            ));
        }
        db.collection::<DurableOperationGrant>(GRANTS)
            .insert_many(&grants)
            .await?;
        let activation = db
            .collection::<ApiKey>(API_KEYS)
            .update_one(
                doc! {
                    "_id": &key.id,
                    "user_id": owner_user_id,
                    "purpose": "scheduled_invocation",
                    "scheduled_write_enabled": false,
                },
                doc! { "$set": { "scheduled_write_enabled": true } },
            )
            .await?;
        if activation.modified_count != 1 {
            return Err(AppError::Internal(
                "scheduled key activation fence could not be committed".to_string(),
            ));
        }
        Ok(())
    }
    .await;

    if let Err(error) = provision_result {
        let now = Utc::now();
        let _ = db
            .collection::<ApiKey>(API_KEYS)
            .update_one(
                doc! { "_id": &key.id },
                doc! { "$set": {
                    "is_active": false,
                    "scheduled_write_enabled": false,
                    "provisioning_failed_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await;
        return Err(error);
    }

    let mut key = key;
    key.scheduled_write_enabled = true;
    Ok(ProvisionedScheduledKey { key, grants })
}

#[allow(clippy::too_many_arguments)]
pub async fn reauthorize_scheduled_key(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    actor_user_id: &str,
    owner_user_id: &str,
    api_key_id: &str,
    selections: &[DurableOperationSelection],
    expected_digest: &str,
) -> AppResult<Vec<DurableOperationGrant>> {
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {
            "_id": api_key_id,
            "user_id": owner_user_id,
            "is_active": true,
            "purpose": "scheduled_invocation",
        })
        .await?
        .ok_or_else(|| {
            AppError::DurableGrantMissing("scheduled_invocation key not found".to_string())
        })?;
    let expires_at = key.expires_at.ok_or_else(|| {
        AppError::DurableGrantMismatch("scheduled_invocation key has no finite expiry".to_string())
    })?;
    let plan = api_key_scope_service::verify_durable_scope_plan_precondition(
        db,
        node_ws_manager,
        actor_user_id,
        owner_user_id,
        &key.allowed_service_ids,
        &key.allowed_node_ids,
        selections,
        expires_at,
        expected_digest,
    )
    .await?;
    let existing = list_grants(db, owner_user_id, api_key_id, false).await?;
    let previous_by_operation: BTreeMap<(&str, &str), &str> = existing
        .iter()
        .map(|grant| {
            (
                (grant.user_service_id.as_str(), grant.endpoint_id.as_str()),
                grant.id.as_str(),
            )
        })
        .collect();
    let grants: Vec<DurableOperationGrant> = plan
        .durable_operations
        .iter()
        .map(|operation| {
            grant_from_plan(
                owner_user_id,
                api_key_id,
                actor_user_id,
                operation,
                previous_by_operation
                    .get(&(
                        operation.user_service_id.as_str(),
                        operation.endpoint_id.as_str(),
                    ))
                    .copied(),
            )
        })
        .collect::<AppResult<_>>()?;
    if grants.is_empty() {
        return Err(validation(
            "reauthorization requires at least one durable operation",
        ));
    }
    let now = Utc::now();
    db.collection::<DurableOperationGrant>(GRANTS)
        .update_many(
            doc! {
                "user_id": owner_user_id,
                "api_key_id": api_key_id,
                "_id": { "$nin": grants.iter().map(|grant| &grant.id).collect::<Vec<_>>() },
                "revoked_at": bson::Bson::Null,
            },
            doc! { "$set": {
                "revoked_at": bson::DateTime::from_chrono(now),
                "revoked_by": actor_user_id,
                "updated_at": bson::DateTime::from_chrono(now),
            }, "$inc": { "state_version": 1_i64 } },
        )
        .await?;
    // Reauthorization is an intentionally fail-closed saga: old authority is
    // revoked before replacements are inserted. A database failure can cause
    // temporary loss of authority, but can never leave old and new grants
    // active together.
    db.collection::<DurableOperationGrant>(GRANTS)
        .insert_many(&grants)
        .await?;
    Ok(grants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, test_helpers::dummy_service,
    };
    use crate::models::service_endpoint::{
        COLLECTION_NAME as SERVICE_ENDPOINTS, OperationResponseContract,
    };
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::test_utils::{connect_test_database, test_user_endpoint, test_user_service};
    use mongodb::{IndexModel, options::IndexOptions};
    use serde_json::json;

    fn exact(value: Value) -> DurableParameterConstraint {
        DurableParameterConstraint {
            required: true,
            rule: DurableValueConstraint::Exact { value },
        }
    }

    fn endpoint() -> ServiceEndpoint {
        let now = Utc::now();
        ServiceEndpoint {
            id: Uuid::new_v4().to_string(),
            service_id: Uuid::new_v4().to_string(),
            name: "create_item".to_string(),
            description: None,
            method: "POST".to_string(),
            path: "/items/{item_id}".to_string(),
            parameters: Some(json!([
                {"name": "item_id", "in": "path", "required": true},
                {"name": "mode", "in": "query", "required": true}
            ])),
            request_body_schema: Some(json!({"type": "object"})),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
            response_description: None,
            response: OperationResponseContract::default(),
            risk: Some(EndpointRisk::Write),
            supports_idempotency_key: false,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn constraints() -> DurableOperationConstraints {
        DurableOperationConstraints {
            path: BTreeMap::from([("item_id".to_string(), exact(json!("42")))]),
            query: BTreeMap::from([("mode".to_string(), exact(json!("sync")))]),
            headers: BTreeMap::new(),
            body: Some(DurableBodyConstraint {
                fields: BTreeMap::from([("".to_string(), exact(json!({"name": "alpha"})))]),
                allow_additional_fields: false,
            }),
        }
    }

    fn scheduled_key(id: &str, owner: &str, user_service_id: &str) -> ApiKey {
        let now = Utc::now();
        ApiKey {
            id: id.to_string(),
            user_id: owner.to_string(),
            name: "scheduler".to_string(),
            key_prefix: "nyxid_ag_test".to_string(),
            key_hash: format!("hash-{id}"),
            scopes: "proxy".to_string(),
            last_used_at: None,
            expires_at: Some(now + Duration::hours(2)),
            is_active: true,
            created_at: now,
            rotation_predecessor_id: None,
            state_version: 1,
            updated_at: Some(now),
            description: None,
            allowed_service_ids: vec![user_service_id.to_string()],
            allowed_node_ids: Vec::new(),
            allow_all_services: false,
            allow_all_nodes: false,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: Some("generic".to_string()),
            callback_url: None,
            purpose: ApiKeyPurpose::ScheduledInvocation,
            scheduled_write_enabled: true,
        }
    }

    fn grant(
        id: &str,
        owner: &str,
        key_id: &str,
        user_service_id: &str,
        endpoint: &ServiceEndpoint,
    ) -> DurableOperationGrant {
        let now = Utc::now();
        let mut published_endpoint = endpoint.clone();
        published_endpoint.service_id = user_service_id.to_string();
        DurableOperationGrant {
            id: id.to_string(),
            user_id: owner.to_string(),
            api_key_id: key_id.to_string(),
            user_service_id: user_service_id.to_string(),
            endpoint_id: endpoint.id.clone(),
            method: "POST".to_string(),
            normalized_path_template: "/items/{item_id}".to_string(),
            contract_digest: endpoint_contract_digest(&published_endpoint).unwrap(),
            constraints: constraints(),
            valid_from: now - Duration::minutes(1),
            expires_at: now + Duration::hours(1),
            total_limit: 10,
            total_used: 0,
            window: None,
            window_started_at: None,
            window_used: 0,
            replay_policy: DurableReplayPolicy::NonReplayable,
            client_audit_binding: None,
            revoked_at: None,
            revoked_by: None,
            state_version: 1,
            created_by: owner.to_string(),
            reauthorized_from: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn endpoint_digest_changes_for_security_relevant_contract_drift() {
        let original = endpoint();
        let original_digest = endpoint_contract_digest(&original).unwrap();

        let mut changed = original.clone();
        changed.supports_idempotency_key = true;
        assert_ne!(original_digest, endpoint_contract_digest(&changed).unwrap());

        changed = original.clone();
        changed.request_body_schema = Some(json!({"type": "object", "required": ["name"]}));
        assert_ne!(original_digest, endpoint_contract_digest(&changed).unwrap());

        changed = original;
        changed.risk = Some(EndpointRisk::Read);
        assert_ne!(original_digest, endpoint_contract_digest(&changed).unwrap());
    }

    #[test]
    fn constraints_fail_closed_for_unbounded_path_body_and_system_headers() {
        let endpoint = endpoint();
        let mut input = constraints();
        input.path.clear();
        assert!(normalize_and_validate_constraints(&endpoint, &input).is_err());

        let body_error = validate_body_constraint(
            &endpoint,
            constraints().body.as_ref(),
            br#"{"name":"alpha","extra":true}"#,
        );
        assert!(matches!(body_error, Err(AppError::DurableGrantMismatch(_))));

        let mut input = constraints();
        input.headers.insert(
            "Idempotency-Key".to_string(),
            exact(json!("caller-controlled")),
        );
        assert!(normalize_and_validate_constraints(&endpoint, &input).is_err());

        let mut input = constraints();
        input
            .body
            .as_mut()
            .unwrap()
            .fields
            .insert("/name".to_string(), exact(json!("alpha")));
        assert!(normalize_and_validate_constraints(&endpoint, &input).is_err());

        let mut input = constraints();
        input.body.as_mut().unwrap().fields = BTreeMap::from([
            ("/object".to_string(), exact(json!({"leaf": true}))),
            ("/object/leaf".to_string(), exact(json!(true))),
        ]);
        assert!(normalize_and_validate_constraints(&endpoint, &input).is_err());

        let mut endpoint_with_header = endpoint.clone();
        endpoint_with_header.parameters = Some(json!([
            {"name": "item_id", "in": "path", "required": true},
            {"name": "mode", "in": "query", "required": true},
            {"name": "X-Amz-Target", "in": "header", "required": false}
        ]));
        let mut headers = HeaderMap::new();
        headers.insert("x-amz-target", "Different.Operation".parse().unwrap());
        let actual = constrained_headers(&endpoint_with_header, &headers).unwrap();
        assert!(matches!(
            validate_parameter_constraints(&BTreeMap::new(), &actual, "header"),
            Err(AppError::DurableGrantMismatch(_))
        ));

        let mut undeclared_operation_header = HeaderMap::new();
        undeclared_operation_header.insert("x-amz-target", "Different.Operation".parse().unwrap());
        assert!(matches!(
            constrained_headers(&endpoint, &undeclared_operation_header),
            Err(AppError::DurableGrantMismatch(_))
        ));
    }

    #[test]
    fn operation_ids_are_bounded_ascii_identifiers() {
        assert!(valid_operation_id("schedule-42:call_site_3"));
        assert!(!valid_operation_id(""));
        assert!(!valid_operation_id("contains space"));
        assert!(!valid_operation_id(&"a".repeat(MAX_OPERATION_ID_LEN + 1)));
    }

    #[tokio::test]
    async fn operation_plans_reject_template_rows_hidden_by_an_instance_spec() {
        let Some(db) = connect_test_database("durable_instance_spec_precedence").await else {
            return;
        };
        let _cache_guard = crate::services::api_docs_service::SpecCacheTestGuard::acquire();
        let owner = Uuid::new_v4().to_string();
        let user_service_id = Uuid::new_v4().to_string();
        let user_endpoint_id = Uuid::new_v4().to_string();
        let template = endpoint();
        let spec_url = format!("https://example.com/{}.json", Uuid::new_v4());

        let mut catalog_service = dummy_service();
        catalog_service.id = template.service_id.clone();
        catalog_service.slug = format!("durable-precedence-{}", Uuid::new_v4());
        catalog_service.requires_user_credential = false;
        db.collection::<crate::models::downstream_service::DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog_service)
            .await
            .unwrap();
        db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .insert_one(&template)
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &user_endpoint_id,
                &owner,
                "Instance override",
                "https://durable.example.test",
                Some(&spec_url),
                Some(&template.service_id),
            ))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &user_service_id,
                &owner,
                "durable-instance-override",
                &user_endpoint_id,
                Some(&template.service_id),
                None,
            ))
            .await
            .unwrap();
        crate::services::api_docs_service::cache_test_spec(
            &spec_url,
            Some(&owner),
            json!({
                "openapi": "3.1.0",
                "info": { "title": "Instance", "version": "1.0.0" },
                "paths": {
                    "/instance-write": {
                        "post": {
                            "operationId": "instance_write",
                            "x-aevatar-tool": { "readOnly": false },
                            "responses": { "200": { "description": "ok" } }
                        }
                    }
                }
            }),
        );
        let cached_spec =
            crate::services::api_docs_service::fetch_spec_json_scoped(&spec_url, &owner)
                .await
                .unwrap();
        let parsed_spec =
            crate::services::openapi_parser::parse_openapi_spec_value(&cached_spec).unwrap();
        assert_eq!(parsed_spec.len(), 1);
        assert_eq!(parsed_spec[0].risk, Some(EndpointRisk::Write));

        let now = Utc::now();
        let selection = DurableOperationSelection {
            user_service_id: user_service_id.clone(),
            endpoint_id: template.id.clone(),
            constraints: constraints(),
            valid_from: now.to_rfc3339(),
            expires_at: (now + Duration::hours(1)).to_rfc3339(),
            total_limit: 1,
            window: None,
            replay_policy: DurableReplayPolicy::NonReplayable,
            client_audit_binding: None,
        };
        let result = build_operation_plans(
            &db,
            &NodeWsManager::new(30, 100),
            &owner,
            &[],
            &[selection],
            Some(now + Duration::hours(2)),
        )
        .await;
        assert!(
            matches!(result, Err(AppError::ApiKeyScopePlanNotFound(_))),
            "unexpected planning result: {result:?}"
        );
    }

    #[tokio::test]
    async fn exact_key_binding_and_operation_ledger_are_fail_closed() {
        let Some(db) = connect_test_database("durable_operation_ledger").await else {
            return;
        };
        db.collection::<mongodb::bson::Document>(EXECUTIONS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "grant_id": 1, "operation_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .unwrap();

        let owner = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();
        let other_key_id = Uuid::new_v4().to_string();
        let user_service_id = Uuid::new_v4().to_string();
        let user_endpoint_id = Uuid::new_v4().to_string();
        let endpoint = endpoint();
        let mut grant = grant(
            &Uuid::new_v4().to_string(),
            &owner,
            &key_id,
            &user_service_id,
            &endpoint,
        );
        grant
            .constraints
            .body
            .as_mut()
            .unwrap()
            .fields
            .get_mut("")
            .unwrap()
            .rule = DurableValueConstraint::OneOf {
            values: vec![json!({"name": "alpha"}), json!({"name": "different"})],
        };
        let mut catalog_service = dummy_service();
        catalog_service.id = endpoint.service_id.clone();
        catalog_service.slug = format!("durable-ledger-{}", Uuid::new_v4());
        catalog_service.requires_user_credential = false;
        db.collection::<crate::models::downstream_service::DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog_service)
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &user_endpoint_id,
                &owner,
                "Durable ledger endpoint",
                "https://durable.example.test",
                None,
                Some(&endpoint.service_id),
            ))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &user_service_id,
                &owner,
                "durable-ledger-service",
                &user_endpoint_id,
                Some(&endpoint.service_id),
                None,
            ))
            .await
            .unwrap();
        db.collection::<ApiKey>(API_KEYS)
            .insert_many([
                scheduled_key(&key_id, &owner, &user_service_id),
                scheduled_key(&other_key_id, &owner, &user_service_id),
            ])
            .await
            .unwrap();
        db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .insert_one(&endpoint)
            .await
            .unwrap();
        db.collection::<DurableOperationGrant>(GRANTS)
            .insert_one(&grant)
            .await
            .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        let node_ws_manager = NodeWsManager::new(30, 100);
        let wrong_key = authorize_and_reserve(
            &db,
            &node_ws_manager,
            &owner,
            &other_key_id,
            &user_service_id,
            "POST",
            "/items/42",
            Some("mode=sync"),
            &headers,
            br#"{"name":"alpha"}"#,
            &grant.id,
            "wrong-key",
            false,
        )
        .await;
        assert!(matches!(wrong_key, Err(AppError::DurableGrantMismatch(_))));
        assert_eq!(
            db.collection::<DurableOperationExecution>(EXECUTIONS)
                .count_documents(doc! { "operation_id": "wrong-key" })
                .await
                .unwrap(),
            0
        );

        let invoke = || {
            authorize_and_reserve(
                &db,
                &node_ws_manager,
                &owner,
                &key_id,
                &user_service_id,
                "POST",
                "/items/42",
                Some("mode=sync"),
                &headers,
                br#"{"name":"alpha"}"#,
                &grant.id,
                "same-operation",
                false,
            )
        };
        let (left, right) = tokio::join!(invoke(), invoke());
        let reservations: Vec<_> = [left, right].into_iter().filter_map(Result::ok).collect();
        assert_eq!(reservations.len(), 1);

        let duplicate = authorize_and_reserve(
            &db,
            &node_ws_manager,
            &owner,
            &key_id,
            &user_service_id,
            "POST",
            "/items/42",
            Some("mode=sync"),
            &headers,
            br#"{"name":"alpha"}"#,
            &grant.id,
            "same-operation",
            false,
        )
        .await;
        assert!(matches!(
            duplicate,
            Err(AppError::DurableOperationDuplicate)
        ));

        let conflict = authorize_and_reserve(
            &db,
            &node_ws_manager,
            &owner,
            &key_id,
            &user_service_id,
            "POST",
            "/items/42",
            Some("mode=sync"),
            &headers,
            br#"{"name":"different"}"#,
            &grant.id,
            "same-operation",
            false,
        )
        .await;
        assert!(matches!(conflict, Err(AppError::DurableOperationConflict)));

        mark_dispatched(&db, &reservations[0], None).await.unwrap();
        let uncertain = authorize_and_reserve(
            &db,
            &node_ws_manager,
            &owner,
            &key_id,
            &user_service_id,
            "POST",
            "/items/42",
            Some("mode=sync"),
            &headers,
            br#"{"name":"alpha"}"#,
            &grant.id,
            "same-operation",
            false,
        )
        .await;
        assert!(matches!(
            uncertain,
            Err(AppError::DurableOperationOutcomeUncertain)
        ));

        let stored = db
            .collection::<DurableOperationGrant>(GRANTS)
            .find_one(doc! { "_id": &grant.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.total_used, 1);
    }
}
