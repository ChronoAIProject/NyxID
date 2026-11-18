//! Bounded native operation discovery. REST retains its full management view.
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::ServiceOptions;
use crate::errors::{AppError, AppResult};

// Budget the MCP text envelope too: NyxAgent serializes it again before its
// 10,000-character projection. Bytes are conservative for Unicode text.
const MAX_WIRE_BYTES: usize = 9_200;
const DEFAULT_LIMIT: usize = 20;

#[derive(Default, Deserialize)]
pub(crate) struct PageRequest {
    #[serde(default)]
    pub guests: bool,
    pub service_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

fn invalid_cursor() -> AppError {
    AppError::ValidationError(
        "Invalid discovery cursor; restart get_agent_operations without cursor".into(),
    )
}

fn width(service: &Value) -> usize {
    ["operations", "endpoint_ids", "rules", "inputs"]
        .iter()
        .map(|field| match &service[field] {
            Value::Array(rows) => rows.len(),
            Value::Object(rows) => rows.len(),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
        .max(1)
}

fn slice(service: &Value, start: usize, count: usize) -> Value {
    let mut result = service.clone();
    for field in ["operations", "endpoint_ids", "rules"] {
        result[field] = Value::Array(
            service[field]
                .as_array()
                .expect("typed service array")
                .iter()
                .skip(start)
                .take(count)
                .cloned()
                .collect(),
        );
    }
    result["inputs"] = Value::Object(
        service["inputs"]
            .as_object()
            .expect("typed input rules")
            .iter()
            .skip(start)
            .take(count)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    result["page"] = json!({
        "offset":start, "count":count, "total":width(service),
        "complete":start == 0 && count == width(service),
    });
    result
}

fn projection(services: &[Value], snapshot: &str, start: usize, count: usize) -> Value {
    let total: usize = services.iter().map(width).sum();
    let end = start + count;
    let mut offset = 0;
    let mut rows = Vec::new();
    for service in services {
        let next = offset + width(service);
        if start < next && end > offset {
            let local_start = start.saturating_sub(offset);
            rows.push(slice(
                service,
                local_start,
                end.min(next) - offset - local_start,
            ));
        }
        offset = next;
    }
    let complete = end == total;
    json!({
        "services":rows, "snapshot":snapshot, "offset":start, "count":count,
        "total":total, "complete":complete, "truncated":!complete,
        "next_cursor":(!complete).then(|| format!("v1:{snapshot}:{end}")),
        "continuation_hint":(!complete).then_some(
            "Call get_agent_operations with cursor=next_cursor and the same subagent, service_id and guests; limit may be reduced. Append operations, endpoint_ids and rules and merge inputs per service until complete=true before changing a selection."
        ),
    })
}

fn wire_bytes(value: &Value) -> usize {
    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
        .to_string()
        .len()
}

pub(super) fn page(
    mut options: Vec<ServiceOptions>,
    binding: &Value,
    request: &PageRequest,
) -> AppResult<Value> {
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=50).contains(&limit) {
        return Err(AppError::ValidationError("Use limit from 1 to 50".into()));
    }
    options.sort_by(|a, b| a.service_id.cmp(&b.service_id));
    for service in &mut options {
        service
            .operations
            .sort_by(|a, b| a.endpoint_id.cmp(&b.endpoint_id));
        service.endpoint_ids.sort();
        // Rule order has no authority semantics; serialize for a total order
        // including parameter grammars, not just the displayed method/path.
        service
            .rules
            .sort_by_cached_key(|rule| serde_json::to_string(rule).expect("typed rule"));
    }
    let services: Vec<Value> = options
        .into_iter()
        .map(|row| serde_json::to_value(row).expect("typed service options"))
        .collect();
    let versioned = json!([
        "operation-discovery-v1",
        binding,
        request.service_id,
        request.guests,
        services
    ]);
    let snapshot = hex::encode(Sha256::digest(versioned.to_string().as_bytes()));
    let total: usize = services.iter().map(width).sum();
    let start = if let Some(cursor) = &request.cursor {
        if cursor.len() > 128 {
            return Err(invalid_cursor());
        }
        let mut parts = cursor.split(':');
        if parts.next() != Some("v1") {
            return Err(invalid_cursor());
        }
        let digest = parts.next().ok_or_else(invalid_cursor)?;
        let offset = parts.next().ok_or_else(invalid_cursor)?;
        if parts.next().is_some()
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            || offset.is_empty()
            || !offset.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid_cursor());
        }
        if digest != snapshot {
            return Err(AppError::Conflict(
                "Operation discovery changed or cursor belongs to another caller/filter; discard collected pages and restart get_agent_operations without cursor".into(),
            ));
        }
        let start: usize = offset.parse().map_err(|_| invalid_cursor())?;
        if start == 0 || start >= total {
            return Err(invalid_cursor());
        }
        start
    } else {
        0
    };
    let mut count = limit.min(total - start);
    loop {
        let result = projection(&services, &snapshot, start, count);
        if wire_bytes(&result) <= MAX_WIRE_BYTES {
            // Metadata only: never log paths, summaries, input values or cursor.
            tracing::debug!(
                operation_discovery_rows = total,
                page_rows = count,
                result_bytes = result.to_string().len(),
                wire_bytes = wire_bytes(&result),
                "Operation discovery page"
            );
            return Ok(result);
        }
        if count <= 1 {
            return Err(AppError::ValidationError(
                "A discovery entry exceeds the page budget. Inspect the agent's Operations settings in the web app; no complete selection was returned. Do not guess IDs or widen access.".into(),
            ));
        }
        count /= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::agent_operation_scope_service::OperationOption;

    fn catalog(count: usize) -> Vec<ServiceOptions> {
        vec![ServiceOptions {
            service_id: "service".into(),
            service_slug: "catalog".into(),
            service_name: "目录".into(),
            revision: 4,
            all_operations: false,
            allows_explicit_rules: false,
            endpoint_ids: (0..count).map(|i| format!("endpoint-{i:04}")).collect(),
            rules: vec![],
            inputs: Default::default(),
            operations: (0..count)
                .rev()
                .map(|i| OperationOption {
                    endpoint_id: format!("endpoint-{i:04}"),
                    method: "GET".into(),
                    path: format!("/items/{i}"),
                    summary: Some("中文 JSON \"value\"\n# Markdown\\path".repeat(5)),
                    read_only: true,
                    changes_existing: false,
                })
                .collect(),
        }]
    }

    #[test]
    fn assistant_team_discovery_pages_preserve_every_id_and_fit_escaped_budget() {
        let binding = json!(["owner", "key", "thread", "agent"]);
        let mut request = PageRequest {
            limit: Some(50),
            ..Default::default()
        };
        let mut ids = Vec::new();
        let mut selected = Vec::new();
        let raw = json!({"services":catalog(300)});
        assert!(wire_bytes(&raw) > 100_000);
        let mut pages = 0;
        loop {
            let result = page(catalog(300), &binding, &request).unwrap();
            assert!(wire_bytes(&result) <= MAX_WIRE_BYTES);
            ids.extend(
                result["services"][0]["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| o["endpoint_id"].as_str().unwrap().to_owned()),
            );
            selected.extend(
                result["services"][0]["endpoint_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .cloned(),
            );
            pages += 1;
            request.cursor = result["next_cursor"].as_str().map(str::to_owned);
            if request.cursor.is_none() {
                assert_eq!(result["complete"], true);
                break;
            }
            assert_eq!(result["truncated"], true);
            assert!(
                result["continuation_hint"]
                    .as_str()
                    .unwrap()
                    .contains("cursor=next_cursor")
            );
        }
        assert!(pages > 6);
        assert_eq!(
            ids,
            (0..300)
                .map(|i| format!("endpoint-{i:04}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(selected, ids.iter().map(|s| json!(s)).collect::<Vec<_>>());
    }

    #[test]
    fn assistant_team_discovery_cursor_is_bound_to_view_and_identity() {
        let binding = json!(["owner", "key", "thread", "agent", "target"]);
        let first = page(
            catalog(3),
            &binding,
            &PageRequest {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let request = PageRequest {
            cursor: Some(first["next_cursor"].as_str().unwrap().into()),
            ..Default::default()
        };
        assert!(page(catalog(3), &binding, &request).is_ok());
        let mut reordered = catalog(3);
        reordered[0].operations.reverse();
        reordered[0].endpoint_ids.reverse();
        assert!(page(reordered, &binding, &request).is_ok());
        for changed in [
            json!(["other", "key", "thread", "agent", "target"]),
            json!(["owner", "other", "thread", "agent", "target"]),
            json!(["owner", "key", "other", "agent", "target"]),
            json!(["owner", "key", "thread", "other", "target"]),
            json!(["owner", "key", "thread", "agent", "other"]),
        ] {
            assert!(matches!(
                page(catalog(3), &changed, &request),
                Err(AppError::Conflict(_))
            ));
        }
        assert!(matches!(
            page(catalog(4), &binding, &request),
            Err(AppError::Conflict(_))
        ));
        let mut changed = catalog(3);
        changed[0].revision += 1;
        assert!(matches!(
            page(changed, &binding, &request),
            Err(AppError::Conflict(_))
        ));
        for changed in [
            PageRequest {
                guests: true,
                ..request_for(&request)
            },
            PageRequest {
                service_id: Some("service".into()),
                ..request_for(&request)
            },
        ] {
            assert!(matches!(
                page(catalog(3), &binding, &changed),
                Err(AppError::Conflict(_))
            ));
        }
        for cursor in ["v0:x:1", "v1:bad:1", "", "v1:x:-1"] {
            assert!(
                page(
                    catalog(3),
                    &binding,
                    &PageRequest {
                        cursor: Some(cursor.into()),
                        ..Default::default()
                    }
                )
                .is_err()
            );
        }
    }

    fn request_for(request: &PageRequest) -> PageRequest {
        PageRequest {
            cursor: request.cursor.clone(),
            ..Default::default()
        }
    }

    #[test]
    fn assistant_team_discovery_pages_preserve_saved_rules_and_inputs_across_services() {
        let source = || {
            let mut rows = catalog(4);
            rows[0].operations.truncate(1);
            rows[0].inputs = (0..7)
                .map(|i| {
                    (
                        format!("endpoint-{i:04}"),
                        nyxid_permissions::values::InputRules {
                            path: std::collections::BTreeMap::from([(
                                "item".into(),
                                nyxid_permissions::ValueRule::Exact {
                                    value: json!(format!("private-{i}")),
                                },
                            )]),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            rows[0].rules = (0..9)
                .map(|i| crate::models::downstream_service::ProxyOperationRule {
                    method: "GET".into(),
                    path_template: format!("/rules/{i}"),
                    ..Default::default()
                })
                .collect();
            let mut empty = catalog(0).remove(0);
            empty.service_id = "z-empty".into();
            rows.push(empty);
            rows
        };
        let expected = source().remove(0);
        let mut request = PageRequest {
            limit: Some(2),
            ..Default::default()
        };
        let mut operations = Vec::new();
        let mut ids = Vec::new();
        let mut rules = Vec::new();
        let mut inputs = serde_json::Map::new();
        let mut empty_count = 0;
        loop {
            let value = page(source(), &Value::Null, &request).unwrap();
            for service in value["services"].as_array().unwrap() {
                if service["service_id"] == "z-empty" {
                    empty_count += 1;
                    assert_eq!(service["page"]["complete"], true);
                    continue;
                }
                operations.extend(service["operations"].as_array().unwrap().iter().cloned());
                ids.extend(service["endpoint_ids"].as_array().unwrap().iter().cloned());
                rules.extend(service["rules"].as_array().unwrap().iter().cloned());
                for (id, rule) in service["inputs"].as_object().unwrap() {
                    assert!(inputs.insert(id.clone(), rule.clone()).is_none());
                }
                assert_eq!(service["page"]["complete"], false);
            }
            request.cursor = value["next_cursor"].as_str().map(str::to_owned);
            if request.cursor.is_none() {
                break;
            }
        }
        assert_eq!(empty_count, 1);
        assert_eq!(json!(operations), json!(expected.operations));
        assert_eq!(json!(ids), json!(expected.endpoint_ids));
        assert_eq!(json!(rules), json!(expected.rules));
        assert_eq!(json!(inputs), json!(expected.inputs));
    }

    #[test]
    fn assistant_team_discovery_empty_and_oversized_entries_are_explicit() {
        let empty = page(vec![], &Value::Null, &PageRequest::default()).unwrap();
        assert_eq!(empty["complete"], true);
        assert_eq!(empty["services"], json!([]));
        let legacy = page(catalog(0), &Value::Null, &PageRequest::default()).unwrap();
        assert_eq!(legacy["services"][0]["page"]["complete"], true);
        let mut huge = catalog(1);
        huge[0].operations[0].summary = Some("x".repeat(10_000));
        assert!(matches!(
            page(huge, &Value::Null, &PageRequest::default()),
            Err(AppError::ValidationError(_))
        ));
        for limit in [0, 51, usize::MAX] {
            assert!(
                page(
                    catalog(1),
                    &Value::Null,
                    &PageRequest {
                        limit: Some(limit),
                        ..Default::default()
                    }
                )
                .is_err()
            );
        }
    }
}
