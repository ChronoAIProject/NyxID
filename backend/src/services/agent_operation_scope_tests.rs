use super::agent_operation_scope_service::*;
use crate::models::{agent_operation_scope::*, downstream_service::ProxyOperationRule};
use crate::services::proxy_authorization::CanonicalPath;

fn scope() -> AgentOperationScope {
    AgentOperationScope {
        revision: 1,
        catalog_service_id: Some("catalog".into()),
        operations: vec![ScopedOperation {
            endpoint_id: Some("read-item".into()),
            rule: ProxyOperationRule {
                method: "GET".into(),
                path_template: "/items/{id}".into(),
                ..Default::default()
            },
            ..Default::default()
        }],
    }
}

#[test]
fn assistant_operation_scope_refusal_is_bounded_and_keeps_endpoint_ids() {
    let message = |operations| {
        let scopes = OperationScopes::from([(
            "service".into(),
            AgentOperationScope {
                operations,
                ..scope()
            },
        )]);
        let path = CanonicalPath::from_mcp_literal("/denied").unwrap();
        let Err(crate::errors::AppError::ApiKeyScopeForbidden(message)) = authorize(
            &scopes, "service", None, None, "DELETE", &path, false, false,
        ) else {
            panic!("expected an operation scope refusal")
        };
        assert!(message.len() <= 2048);
        assert!(message.starts_with("Specialist operation scope denied this call."));
        assert!(message.contains("nyx__search_tools"));
        message
    };
    let operations: Vec<_> = (0..256)
        .map(|index| ScopedOperation {
            endpoint_id: Some(format!("endpoint-{index}")),
            rule: ProxyOperationRule {
                method: "GET".into(),
                path_template: format!("/items/{index}"),
                ..Default::default()
            },
            ..Default::default()
        })
        .collect();
    let text = message(operations.clone());
    assert!(text.contains("(endpoint-0)"));
    assert!(text.contains("(endpoint-19)"));
    assert!(!text.contains("(endpoint-20)"));
    assert!(text.contains("and 236 more"));
    let mut long = operations;
    long[0].rule.path_template = format!("/{}", "界".repeat(2048));
    let text = message(long);
    assert!(text.contains("... (endpoint-0)"));
    assert!(text.contains("and 255 more"));
    assert!(message(vec![]).contains("Allowed operations: none."));
}

#[test]
fn assistant_operation_scope_exact_methods_case_segments_and_identity() {
    let scopes = OperationScopes::from([("service".into(), scope())]);
    let path = CanonicalPath::from_mcp_literal("/items/123").unwrap();
    assert_eq!(
        authorize(
            &scopes,
            "service",
            None,
            Some("read-item"),
            "GET",
            &path,
            false,
            false
        )
        .unwrap(),
        Some("items/123".into())
    );
    for method in ["HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "get"] {
        assert!(authorize(&scopes, "service", None, None, method, &path, false, false).is_err());
    }
    assert!(
        authorize(
            &scopes,
            "service",
            None,
            Some("other"),
            "GET",
            &path,
            false,
            false
        )
        .is_err()
    );
    for value in ["/Items/123", "/items/123/edit", "/items"] {
        let path = CanonicalPath::from_mcp_literal(value).unwrap();
        assert!(authorize(&scopes, "service", None, None, "GET", &path, false, false).is_err());
    }
    assert!(authorize(&scopes, "service", None, None, "GET", &path, true, false).is_err());
    assert!(authorize(&scopes, "service", None, None, "GET", &path, false, true).is_err());
}

#[test]
fn assistant_operation_scope_canonicalization_refuses_bypass_spellings() {
    for value in [
        "/items/a%2fb",
        "/items/a%2Fb",
        "/items/%252e%252e",
        "/items/%2e%2e",
        "/items/..",
        "/items/.",
        "/items//123",
        "//items/123",
        "/items/123/",
        "/items/123?method=DELETE",
        "/items/123#x",
        "/items/a\\b",
        "/items/%",
        "/items/%00",
    ] {
        assert!(CanonicalPath::from_mcp_built(value).is_err(), "{value}");
    }
    assert_eq!(
        CanonicalPath::from_mcp_built("/items/%41")
            .unwrap()
            .as_policy_path(),
        "/items/A"
    );
}

#[test]
fn assistant_operation_scope_catalog_alias_pool_member_and_absence() {
    let scopes = OperationScopes::from([("member".into(), scope())]);
    let path = CanonicalPath::from_mcp_literal("/forbidden").unwrap();
    assert!(
        authorize(
            &scopes,
            "member",
            Some("catalog"),
            None,
            "POST",
            &path,
            false,
            false
        )
        .is_err()
    );
    assert!(authorize(&scopes, "catalog", None, None, "POST", &path, false, false).is_err());
    assert_eq!(
        authorize(
            &scopes,
            "unrelated",
            None,
            None,
            "POST",
            &path,
            false,
            false
        )
        .unwrap(),
        None
    );
    let empty = OperationScopes::from([(
        "member".into(),
        AgentOperationScope {
            operations: vec![],
            ..scope()
        },
    )]);
    let path = CanonicalPath::from_mcp_literal("/items/123").unwrap();
    assert!(authorize(&empty, "member", None, None, "GET", &path, false, false).is_err());
}

#[test]
fn assistant_operation_scope_widening_is_conservative_and_deny_empty_is_narrowing() {
    let old = scope();
    let empty = AgentOperationScope {
        operations: vec![],
        ..old.clone()
    };
    assert!(!widens(None, Some(&old)));
    assert!(!widens(Some(&old), Some(&empty)));
    assert!(!widens(Some(&old), Some(&old)));
    let mut narrower = old.clone();
    narrower.operations[0].rule.path_template = "/items/one".into();
    assert!(!widens(Some(&old), Some(&narrower)));
    assert!(widens(Some(&narrower), Some(&old)));
    // Ordinary parameters exclude custom-method delimiters. A literal custom
    // method is a widening even though it occupies the same path segment.
    for path in ["/items/one:delete", "/items/{id}:delete"] {
        let mut custom_method = old.clone();
        custom_method.operations[0].rule.path_template = path.into();
        assert!(widens(Some(&old), Some(&custom_method)));
    }
    assert!(widens(Some(&empty), Some(&old)));
    assert!(widens(Some(&old), None));
    let mut changed = old.clone();
    changed.operations[0].endpoint_id = Some("different-id".into());
    assert!(widens(Some(&old), Some(&changed)));
}

#[test]
fn assistant_operation_scope_reuses_override_detection_for_headers_queries_and_bodies() {
    use axum::http::HeaderMap;
    let mut headers = HeaderMap::new();
    for header in [
        "x-http-method-override",
        "x-method-override",
        "x-http-method",
        "x_method_override",
    ] {
        headers.insert(header, "DELETE".parse().unwrap());
        assert!(crate::services::mcp_service::http_carries_method_override(
            "GET",
            &headers,
            None,
            &[]
        ));
        headers.clear();
    }
    for query in [
        "_method=DELETE",
        "%5fmethod=DELETE",
        "method=DELETE",
        "method=HEAD",
        "x-http-method-override=DELETE",
        "_method[]=PUT",
        "x=1;_HttpMethod=DELETE",
    ] {
        assert!(crate::services::mcp_service::http_carries_method_override(
            "GET",
            &headers,
            Some(query),
            &[]
        ));
    }
    assert!(!crate::services::mcp_service::http_carries_method_override(
        "GET",
        &headers,
        Some("limit=2"),
        &[]
    ));
    headers.insert("content-type", "application/json".parse().unwrap());
    assert!(crate::services::mcp_service::http_carries_method_override(
        "POST",
        &headers,
        None,
        br#"{"_method":"DELETE"}"#
    ));
}

#[tokio::test]
async fn assistant_operation_scope_transaction_sync_revision_regrant_rotation_and_legacy() {
    use crate::models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        assistant_agent::AgentGrants,
    };
    use crate::services::{
        assistant_authority_tests::{connected, fixture},
        assistant_nyxagent as engine, assistant_team_service as team,
    };
    use mongodb::bson::doc;
    let f = fixture("operation_scope_sync").await;
    crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
    let service = connected(&f.state.db, &f.owner, "scope-sync", "http://127.0.0.1:9").await;
    let grants = AgentGrants {
        service_ids: vec![service.clone()],
        ..Default::default()
    };
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(grants.clone()),
    )
    .await
    .unwrap();
    let second = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &engine::TurnRequest {
            agent_id: Some(f.chat.agent_id.clone()),
            conversation_id: None,
            text: "Another thread".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let mut selection = OperationSelection {
        expected_revision: 0,
        all_operations: false,
        endpoint_ids: vec![],
        rules: vec![ProxyOperationRule {
            method: "GET".into(),
            path_template: "/items/{id}".into(),
            ..Default::default()
        }],
    };
    let agent = set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &service,
        &selection,
        false,
    )
    .await
    .unwrap();
    for key in [&f.row.credential_api_key_id, &second.credential_api_key_id] {
        let key = f
            .state
            .db
            .collection::<ApiKey>(KEYS)
            .find_one(doc! {"_id":key})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key.assistant_operation_scopes, agent.operation_scopes);
    }
    assert!(
        set(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &service,
            &selection,
            true
        )
        .await
        .is_err()
    );
    selection.expected_revision = 1;
    selection.all_operations = true;
    selection.rules.clear();
    assert!(
        set(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &service,
            &selection,
            false
        )
        .await
        .is_err()
    );
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Remove(grants.clone()),
    )
    .await
    .unwrap();
    let restored = team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(grants),
    )
    .await
    .unwrap();
    assert_eq!(restored.operation_scopes, agent.operation_scopes);
    let rotated = crate::services::key_service::rotate_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.credential_api_key_id,
    )
    .await
    .unwrap();
    let successor = f
        .state
        .db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":&rotated.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(successor.assistant_operation_scopes, agent.operation_scopes);
    let mut legacy_key = bson::to_document(&successor).unwrap();
    legacy_key.remove("assistant_operation_scopes");
    assert!(
        bson::from_document::<ApiKey>(legacy_key)
            .unwrap()
            .assistant_operation_scopes
            .is_empty()
    );
    let legacy = bson::to_document(&restored).unwrap();
    let mut legacy = legacy;
    legacy.remove("operation_scopes");
    legacy.remove("operation_scope_revisions");
    let decoded: crate::models::assistant_agent::AssistantAgent =
        bson::from_document(legacy).unwrap();
    assert!(decoded.operation_scopes.is_empty());
    let reset = set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &service,
        &selection,
        true,
    )
    .await
    .unwrap();
    assert!(reset.operation_scopes.is_empty());
    assert_eq!(reset.operation_scope_revisions[&service], 2);
    let successor = f
        .state
        .db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":&rotated.id})
        .await
        .unwrap()
        .unwrap();
    assert!(successor.assistant_operation_scopes.is_empty());
    f.state.db.drop().await.unwrap();
}
