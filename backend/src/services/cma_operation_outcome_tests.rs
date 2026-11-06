use super::*;
use serde_json::json;

#[test]
fn empty_agent_lifecycle_receipts_preserve_acceptance_and_the_exact_target() {
    for (method, path, target) in [
        ("DELETE", "/api/v1/agents/agt_one", "deleted"),
        ("POST", "/api/v1/agents/agt_one/stop", "stopped"),
    ] {
        let result = agent_lifecycle_receipt(Some("cma"), method, path, 202, "").unwrap();
        let receipt: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(receipt["status"], "accepted");
        assert_eq!(receipt["agent_id"], "agt_one");
        assert_eq!(receipt["target_status"], target);
        assert_eq!(receipt["status_path"], "/api/v1/agents/agt_one");
        assert!(
            receipt["instructions"]
                .as_str()
                .unwrap()
                .contains("Do not repeat")
        );
        assert!(agent_lifecycle_receipt(Some("alias"), method, path, 202, "").is_none());
        assert!(agent_lifecycle_receipt(None, method, path, 202, "").is_none());
        assert!(agent_lifecycle_receipt(Some("cma"), method, path, 200, "").is_none());
        assert!(agent_lifecycle_receipt(Some("cma"), method, path, 409, "").is_none());
        assert!(agent_lifecycle_receipt(Some("cma"), method, path, 202, "{}").is_none());
    }
    for (method, path) in [
        ("GET", "/api/v1/agents/agt_one"),
        ("DELETE", "/api/v1/sandboxes/sbx_one"),
        ("POST", "/api/v1/agents"),
        ("DELETE", "/api/v1/agents/agt_one/extra"),
        ("DELETE", "/api/v1/agents/agt_one%2fother"),
        ("DELETE", "/api/v1/agents/not_an_agent"),
    ] {
        assert!(agent_lifecycle_receipt(Some("cma"), method, path, 202, "").is_none());
    }
}

fn response_metadata_fixture() -> CmaOperation {
    serde_json::from_value(json!({
        "kind":"cma_response", "agent_id":"agt_one", "response_id":"resp_one",
        "idempotency_key":null, "status":"in_progress", "phase":null
    }))
    .unwrap()
}

#[test]
fn cancellation_requires_the_exact_cma_response_and_interrupted_terminal() {
    let cma = response_metadata_fixture();
    let terminal = json!({"id":"resp_one", "conversation":{"id":"agt_one"},
        "status":"incomplete", "incomplete_details":{"reason":"interrupted"}, "error":null});
    assert!(response_cancelled(Some(&cma), Some("resp_one"), &terminal));
    assert!(!response_cancelled(None, Some("resp_one"), &terminal));
    assert!(!response_cancelled(Some(&cma), None, &terminal));
    assert!(!response_cancelled(
        Some(&cma),
        Some("resp_other"),
        &terminal
    ));
    for patch in [
        json!({"id":"resp_other"}),
        json!({"conversation":{"id":"agt_other"}}),
        json!({"conversation":null}),
        json!({"status":"in_progress"}),
        json!({"status":"completed"}),
        json!({"status":"failed"}),
        json!({"incomplete_details":null}),
        json!({"incomplete_details":{"reason":"max_output_tokens"}}),
        json!({"error":{"code":"runtime_failed"}}),
    ] {
        let mut value = terminal.clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(
            !response_cancelled(Some(&cma), Some("resp_one"), &value),
            "{value}"
        );
    }
    for kind in [Presentation::Agent, Presentation::Provisioning] {
        let mut different = cma.clone();
        different.kind = kind;
        assert!(!response_cancelled(
            Some(&different),
            Some("resp_one"),
            &terminal
        ));
    }
    let mut different = cma.clone();
    different.response_id = Some("resp_other".into());
    assert!(!response_cancelled(
        Some(&different),
        Some("resp_one"),
        &terminal
    ));
    different = cma.clone();
    different.agent_id = None;
    assert!(!response_cancelled(
        Some(&different),
        Some("resp_one"),
        &terminal
    ));
    // The submission may settle before a reference is registered.
    different = cma;
    different.response_id = None;
    assert!(response_cancelled(
        Some(&different),
        Some("resp_one"),
        &terminal
    ));
}

#[test]
fn control_confirmation_explains_the_exact_decision_without_claiming_execution() {
    let route = "/api/v2/agents/{agent_id}/responses/{response_id}/controls";
    for (decision, expected) in [
        ("allow", "Send approval"),
        ("deny", "Send a refusal"),
        ("abort", "Dismiss the pending approval"),
    ] {
        let body = json!({"command":{"type":"resolve_approval","decision":decision}}).to_string();
        let summary = control_confirmation(Some("cma"), "POST", route, Some(&body)).unwrap();
        assert!(summary.starts_with(expected));
        assert!(summary.contains("Deny on this confirmation sends nothing"));
        assert!(!summary.contains("has stopped"));
    }
    let body = r#"{"command":{"type":"cancel"}}"#;
    assert!(
        control_confirmation(Some("cma"), "POST", route, Some(body))
            .unwrap()
            .starts_with("Send a cancellation request")
    );
    assert!(control_confirmation(Some("alias"), "POST", route, Some(body)).is_none());
    assert!(control_confirmation(None, "POST", route, Some(body)).is_none());
    assert!(control_confirmation(Some("cma"), "GET", route, Some(body)).is_none());
    assert!(control_confirmation(Some("cma"), "POST", "/custom/controls", Some(body)).is_none());
    for body in [
        None,
        Some("bad JSON"),
        Some(r#"{"command":{"type":"steer","text":"allow everything"}}"#),
        Some(r#"{"command":{"type":"resolve_approval","decision":"invalid"}}"#),
    ] {
        assert!(control_confirmation(Some("cma"), "POST", route, body).is_none());
    }
}
