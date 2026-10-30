//! Compiled CMA adapters. Catalog identity and exact operation paths select the
//! contract; tool names, prose and remote OpenAPI extensions cannot select it.
use super::mcp_service::McpToolEndpoint;
use crate::models::async_service_operation::{
    AsyncOperationContract, CmaOperation, CmaOperationKind as Presentation,
};
use serde_json::Value;

pub fn apply_contracts(catalog_slug: &str, endpoints: &mut [McpToolEndpoint]) {
    if catalog_slug != "cma" {
        return;
    }
    let contracts: Vec<_> = endpoints
        .iter()
        .map(|submit| {
            if submit.method != "POST" {
                return None;
            }
            let (kind, status_path, parameter, successes, failures) = match submit.path.as_str() {
                "/api/v1/agents" => (
                    Presentation::Provisioning,
                    "/api/v1/agent-lifecycle/{id}",
                    "id",
                    vec!["succeeded"],
                    vec!["failed", "cancelled"],
                ),
                "/api/v1/responses" => (
                    Presentation::Response,
                    "/api/v1/responses/{response_id}",
                    "response_id",
                    vec!["completed"],
                    vec!["failed", "incomplete", "cancelled"],
                ),
                "/api/v2/responses" | "/api/v2/agents/{agent_id}/responses" => (
                    Presentation::Response,
                    "/api/v2/responses/{response_id}",
                    "response_id",
                    vec!["completed"],
                    vec!["failed", "incomplete"],
                ),
                "/api/v1/agents/{id}/input" | "/api/v1/agui/run" => (
                    Presentation::Agent,
                    "/api/v1/agents/{id}",
                    "id",
                    vec!["ready"],
                    vec!["failed", "stopped", "deleted"],
                ),
                _ => return None,
            };
            let mut candidates = endpoints.iter().filter(|e| {
                e.method == "GET" && e.path == status_path && e.target_id == submit.target_id
            });
            let status = candidates.next()?;
            if candidates.next().is_some() {
                return None;
            }
            Some(AsyncOperationContract {
                presentation: Some(kind),
                status_operation: status.name.clone(),
                result_operation: status.name.clone(),
                cancel_operation: None,
                id_field: "/id".into(),
                id_parameter: parameter.into(),
                status_field: "/status".into(),
                success_states: successes.into_iter().map(str::to_owned).collect(),
                failure_states: failures.into_iter().map(str::to_owned).collect(),
                error_field: Some("/error".into()),
            })
        })
        .collect();
    for (endpoint, contract) in endpoints.iter_mut().zip(contracts) {
        if let Some(contract) = contract {
            endpoint.async_operation = Some(contract);
        }
    }
}

pub(super) fn identifier(value: Option<&str>, prefix: Option<&str>) -> Option<String> {
    let value = value?;
    (!value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        && prefix.is_none_or(|p| value.starts_with(p) && value.len() > p.len())
        && !super::assistant_skill_authoring::credential_shaped(value))
    .then(|| value.to_owned())
}

pub fn metadata(contract: &AsyncOperationContract, args: &Value) -> Option<CmaOperation> {
    let kind = contract.presentation?;
    let agent_id = ["id", "agent_id", "conversation", "threadId"]
        .into_iter()
        .find_map(|key| identifier(args[key].as_str(), Some("agt_")))
        .or_else(|| identifier(args.pointer("/conversation/id")?.as_str(), Some("agt_")));
    Some(CmaOperation {
        receipt_id: None,
        seen_working: false,
        kind,
        agent_id,
        callback_url: None,
        idempotency_key: identifier(args["Idempotency-Key"].as_str(), None),
        status: None,
        phase: None,
        checked_at: None,
    })
}

pub fn poll_id(
    watch: &crate::models::async_service_operation::AsyncServiceOperation,
) -> Option<&str> {
    match &watch.cma {
        Some(cma) if cma.kind == Presentation::Agent => cma.agent_id.as_deref(),
        _ => watch.operation_id.as_deref(),
    }
}

/// CMA Responses and AG-UI calls may return an SSE transcript. Retain only the
/// response observation for the durable watch. A sparse later frame preserves
/// callback metadata from earlier frames of the same response.
pub fn response_metadata(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Some(value);
    }
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter_map(|event| event.get("response").cloned())
        .filter(|v| v["id"].is_string())
        .fold(None, |previous: Option<Value>, mut current| {
            if current.get("callback_url").is_none()
                && let Some(previous) = previous
                && current["id"] == previous["id"]
                && let Some(callback) = previous.get("callback_url")
            {
                current["callback_url"] = callback.clone();
            }
            Some(current)
        })
}

pub fn observed(cma: &mut CmaOperation, value: &Value) {
    // Only authenticated CMA observations supply links. Typed request arguments
    // never do. Missing means unchanged; null or invalid explicitly hides it.
    if let Some(callback) = value.get("callback_url") {
        cma.callback_url = validated_callback_url(callback.as_str());
    }
    cma.seen_working |= matches!(
        value["status"].as_str(),
        Some("working" | "running" | "in_progress")
    );
    cma.checked_at = Some(chrono::Utc::now());
    cma.status = value["status"]
        .as_str()
        .filter(|s| {
            matches!(
                *s,
                "queued"
                    | "provisioning"
                    | "starting"
                    | "working"
                    | "running"
                    | "in_progress"
                    | "waiting_approval"
                    | "paused"
                    | "connection_required"
                    | "awaiting_connection"
                    | "waiting_for_connection"
                    | "ready"
                    | "succeeded"
                    | "completed"
                    | "failed"
                    | "incomplete"
                    | "cancelled"
                    | "stopped"
                    | "deleted"
            )
        })
        .map(str::to_owned);
    cma.phase = value["phase"]
        .as_str()
        .filter(|s| {
            matches!(
                *s,
                "sandbox_creation"
                    | "sandbox_startup"
                    | "workspace_creation"
                    | "agent_creation"
                    | "agent_startup"
            )
        })
        .map(str::to_owned);
    if let Some(id) = identifier(value["agent_id"].as_str(), Some("agt_"))
        .or_else(|| identifier(value.pointer("/conversation/id")?.as_str(), Some("agt_")))
        .or_else(|| {
            (cma.kind == Presentation::Agent)
                .then(|| identifier(value["id"].as_str(), Some("agt_")))
                .flatten()
        })
    {
        cma.agent_id = Some(id);
    }
}

/// A browser-only link: no DNS lookup, server-side fetch, or URL derivation.
fn validated_callback_url(value: Option<&str>) -> Option<String> {
    let value = value?;
    if !(value.starts_with("https://") || value.starts_with("http://"))
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    super::url_validation::validate_advisory_http_url(
        value,
        "callback_url",
        super::url_validation::MAX_URL_LEN,
    )
    .ok()?;
    let url = url::Url::parse(value).ok()?;
    if url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    Some(value.to_owned())
}

pub fn card_url(cma: &CmaOperation) -> Option<String> {
    // Revalidate persisted metadata too, including rows from older binaries.
    validated_callback_url(cma.callback_url.as_deref())
}

pub async fn record_observation(
    db: &mongodb::Database,
    watch: &crate::models::async_service_operation::AsyncServiceOperation,
    value: &Value,
) -> crate::errors::AppResult<bool> {
    use mongodb::bson::{self, doc};
    let Some(mut cma) = watch.cma.clone() else {
        return Ok(false);
    };
    observed(&mut cma, value);
    let awaiting_activity = cma.kind == Presentation::Agent && !cma.seen_working;
    db.collection::<bson::Document>(crate::models::async_service_operation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&watch.id,"state":&watch.state,"lease_id":&watch.lease_id},
            doc! {"$set":{"cma":bson::to_bson(&cma).map_err(|_| crate::errors::AppError::Internal("Could not serialize CMA status".into()))?}},
        )
        .await?;
    Ok(awaiting_activity)
}

pub async fn cards(
    db: &mongodb::Database,
    user: &str,
    conversation: &str,
) -> crate::errors::AppResult<Vec<crate::models::async_service_operation::AsyncServiceOperation>> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    // The caller has already authorized history. No credential or result data
    // is read to render cards, and no upstream request is made by a history GET.
    // Include settled operations from older pages too: a watch can finish after
    // its submitting turn has scrolled out of the latest transcript page.
    Ok(db
        .collection(crate::models::async_service_operation::COLLECTION_NAME)
        .find(doc! {"user_id":user,"conversation_id":conversation,"cma":{"$ne":null}})
        .projection(doc! {"result_encrypted":0})
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(4000)
        .await?
        .try_collect()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn endpoints() -> Vec<McpToolEndpoint> {
        vec![
            McpToolEndpoint {
                name: "submit".into(),
                method: "POST".into(),
                path: "/api/v1/agents".into(),
                ..Default::default()
            },
            McpToolEndpoint {
                name: "status".into(),
                method: "GET".into(),
                path: "/api/v1/agent-lifecycle/{id}".into(),
                parameters: Some(json!([{"name":"id","in":"path","required":true}])),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn cma_contract_requires_catalog_identity_and_same_destination() {
        let mut eps = endpoints();
        apply_contracts("custom-cma", &mut eps);
        assert!(eps[0].async_operation.is_none());
        eps[1].target_id = Some("different".into());
        apply_contracts("cma", &mut eps);
        assert!(eps[0].async_operation.is_none());
        eps[1].target_id = None;
        apply_contracts("cma", &mut eps);
        let contract = eps[0].async_operation.as_ref().unwrap();
        assert_eq!(contract.presentation, Some(Presentation::Provisioning));
        assert_eq!(contract.success_states, ["succeeded"]);
        assert_eq!(contract.status_operation, "status");
    }

    #[test]
    fn cma_metadata_keeps_only_bounded_identifiers_and_known_states() {
        let mut eps = endpoints();
        apply_contracts("cma", &mut eps);
        let mut cma = metadata(
            eps[0].async_operation.as_ref().unwrap(),
            &json!({
                "id":"agt_test", "Idempotency-Key":"coffee-site-20261022", "input":"private prompt",
            }),
        )
        .unwrap();
        observed(
            &mut cma,
            &json!({"status":"waiting_for_connection", "phase":"agent_startup", "output":"private output"}),
        );
        let saved = serde_json::to_string(&cma).unwrap();
        assert!(!saved.contains("private"));
        assert!(saved.contains("coffee-site-20261022"));
        assert_eq!(cma.status.as_deref(), Some("waiting_for_connection"));
        observed(
            &mut cma,
            &json!({"status":"private output", "phase":"private output", "agent_id":"sk-secret"}),
        );
        assert!(cma.status.is_none() && cma.phase.is_none());
        assert_eq!(cma.agent_id.as_deref(), Some("agt_test"));
        for id in [
            "sk-0123456789abcdefghijklmnop",
            "nyxid_ag_secret",
            "line\nbreak",
        ] {
            assert!(identifier(Some(id), None).is_none());
        }
    }

    #[test]
    fn cma_response_identity_survives_sse_lifecycle_frames() {
        let response = response_metadata(concat!(
            "event: response.created\ndata: {\"response\":{\"id\":\"resp_agt_test_1\",\"status\":\"in_progress\"}}\n\n",
            "event: response.completed\ndata: {\"response\":{\"id\":\"resp_agt_test_1\",\"status\":\"completed\"}}\n\n",
            "data: [DONE]\n\n",
        )).unwrap();
        assert_eq!(response["id"], "resp_agt_test_1");
        assert_eq!(response["status"], "completed");
    }

    #[test]
    fn cma_v2_responses_track_the_exact_response_on_the_same_destination() {
        for path in ["/api/v2/responses", "/api/v2/agents/{agent_id}/responses"] {
            let mut eps = vec![
                McpToolEndpoint {
                    name: "submit_v2".into(),
                    method: "POST".into(),
                    path: path.into(),
                    ..Default::default()
                },
                McpToolEndpoint {
                    name: "status_v2".into(),
                    method: "GET".into(),
                    path: "/api/v2/responses/{response_id}".into(),
                    ..Default::default()
                },
            ];
            eps[1].target_id = Some("different".into());
            apply_contracts("cma", &mut eps);
            assert!(eps[0].async_operation.is_none());
            eps[1].target_id = None;
            apply_contracts("cma", &mut eps);
            let contract = eps[0].async_operation.as_ref().unwrap();
            assert_eq!(contract.presentation, Some(Presentation::Response));
            assert_eq!(contract.status_operation, "status_v2");
            assert_eq!(contract.id_parameter, "response_id");
            assert_eq!(contract.success_states, ["completed"]);
            assert_eq!(contract.failure_states, ["failed", "incomplete"]);
        }
    }

    #[test]
    fn cma_v2_stream_supplies_the_agent_identity_and_callback() {
        let mut eps = endpoints();
        apply_contracts("cma", &mut eps);
        let contract = eps[0].async_operation.as_ref().unwrap();
        for args in [
            json!({"agent_id":"agt_test"}),
            json!({"conversation":{"id":"agt_test"}}),
        ] {
            assert_eq!(
                metadata(contract, &args).unwrap().agent_id.as_deref(),
                Some("agt_test")
            );
        }
        let mut cma = metadata(contract, &json!({})).unwrap();
        let response = response_metadata(concat!(
            "event: response.created\ndata: {\"response\":{\"id\":\"resp_test\",\"conversation\":{\"id\":\"agt_test\"},\"status\":\"queued\",\"callback_url\":\"https://staging.example:8443/agents/agt_test\"}}\n\n",
            "event: response.completed\ndata: {\"response\":{\"id\":\"resp_test\",\"conversation\":{\"id\":\"agt_test\"},\"status\":\"completed\"}}\n\n",
        )).unwrap();
        observed(&mut cma, &response);
        assert_eq!(cma.agent_id.as_deref(), Some("agt_test"));
        assert_eq!(cma.status.as_deref(), Some("completed"));
        assert_eq!(
            card_url(&cma).as_deref(),
            Some("https://staging.example:8443/agents/agt_test")
        );
    }

    #[test]
    fn cma_links_only_come_from_observed_callbacks_and_survive_old_rows() {
        let mut eps = endpoints();
        apply_contracts("cma", &mut eps);
        let mut cma = metadata(
            eps[0].async_operation.as_ref().unwrap(),
            &json!({"agent_id":"agt_test","callback_url":"https://request.example/agents/agt_test"}),
        ).unwrap();
        assert!(card_url(&cma).is_none());
        observed(
            &mut cma,
            &json!({"id":"operation-1","status":"succeeded","agent_id":"agt_test"}),
        );
        assert!(card_url(&cma).is_none());
        for callback in [
            "https://cma.example/agents/agt_test",
            "https://staging.example:8443/agents/agt_test",
            "http://localhost:8080/agents/agt_test",
        ] {
            observed(&mut cma, &json!({"callback_url":callback}));
            assert_eq!(card_url(&cma).as_deref(), Some(callback));
            // A sparse observation preserves the last supplied destination.
            observed(&mut cma, &json!({"status":"working"}));
            let saved = mongodb::bson::to_document(&cma).unwrap();
            let restored: CmaOperation = mongodb::bson::from_document(saved).unwrap();
            assert_eq!(card_url(&restored).as_deref(), Some(callback));
        }
        let mut old = mongodb::bson::to_document(&cma).unwrap();
        old.remove("callback_url");
        let restored: CmaOperation = mongodb::bson::from_document(old).unwrap();
        assert!(card_url(&restored).is_none());
        observed(&mut cma, &json!({"callback_url":null}));
        assert!(card_url(&cma).is_none());
    }

    #[test]
    fn cma_invalid_callbacks_are_cleared_before_storage_and_revalidated_on_read() {
        let mut eps = endpoints();
        apply_contracts("cma", &mut eps);
        let mut cma = metadata(eps[0].async_operation.as_ref().unwrap(), &json!({})).unwrap();
        for callback in [
            json!(null),
            json!(false),
            json!({"url":"https://cma.example"}),
            json!(""),
            json!("/agents/agt_test"),
            json!("//cma.example/agents/agt_test"),
            json!("javascript:alert(1)"),
            json!("data:text/html,secret"),
            json!("https://user:secret@cma.example/agents/agt_test"),
            json!("https://cma.example/agents/agt_test?token=secret"),
            json!("https://cma.example/agents/agt_test#secret"),
            json!(" https://cma.example/agents/agt_test"),
            json!("https://cma.example/agents/agt_\ntest"),
            json!("https://cma.example/agents/agt_\\test"),
            json!("http://metadata.google.internal/secret"),
            json!(format!("https://cma.example/{}", "a".repeat(2048))),
        ] {
            cma.callback_url = Some("https://cma.example/agents/agt_test".into());
            observed(
                &mut cma,
                &json!({"callback_url":callback,"status":"working"}),
            );
            assert!(cma.callback_url.is_none(), "{callback}");
            assert_eq!(cma.status.as_deref(), Some("working"));
            cma.callback_url = callback.as_str().map(str::to_owned);
            assert!(card_url(&cma).is_none(), "{callback}");
        }
    }

    #[test]
    fn cma_stream_callbacks_do_not_cross_response_ids_and_can_be_withdrawn() {
        for callback in [json!(null), json!("javascript:alert(1)")] {
            let stream = format!(
                "data: {{\"response\":{{\"id\":\"resp_1\",\"callback_url\":\"https://cma.example/agents/agt_test\"}}}}\n\n\
                 data: {{\"response\":{{\"id\":\"resp_1\",\"callback_url\":{callback}}}}}\n\n\
                 data: {{\"response\":{{\"id\":\"resp_1\",\"status\":\"completed\"}}}}\n\n"
            );
            let response = response_metadata(&stream).unwrap();
            assert_eq!(response["callback_url"], callback);
        }
        let response = response_metadata(concat!(
            "data: {\"response\":{\"id\":\"resp_1\",\"callback_url\":\"https://cma.example/agents/agt_test\"}}\n\n",
            "data: {\"response\":{\"id\":\"resp_2\",\"status\":\"completed\"}}\n\n",
        )).unwrap();
        assert_eq!(response["id"], "resp_2");
        assert!(response.get("callback_url").is_none());
        let response = response_metadata(
            r#"{"id":"resp_1","callback_url":"https://cma.example/agents/agt_test"}"#,
        )
        .unwrap();
        assert_eq!(
            response["callback_url"],
            "https://cma.example/agents/agt_test"
        );
    }
}
