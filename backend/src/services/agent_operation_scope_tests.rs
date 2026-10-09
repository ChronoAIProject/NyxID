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
            attachment_ids: Vec::new(),
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
        contract_digest: None,
        inputs: Default::default(),
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

fn op(template: &str) -> ScopedOperation {
    ScopedOperation {
        endpoint_id: Some("endpoint".into()),
        rule: ProxyOperationRule {
            method: "GET".into(),
            path_template: template.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn single(operation: ScopedOperation) -> AgentOperationScope {
    AgentOperationScope {
        revision: 1,
        catalog_service_id: None,
        operations: vec![operation],
    }
}

#[test]
fn widening_compares_the_whole_compiled_contract() {
    use crate::models::downstream_service::ProxyPathConstraint;
    // A parameter grammar change on the same endpoint can match wider paths.
    let plain = single(op("/files/{path}"));
    let mut multi = plain.clone();
    multi.operations[0]
        .rule
        .path_parameter_constraints
        .insert("path".into(), ProxyPathConstraint::MultiSegment);
    assert!(widens(Some(&plain), Some(&multi)));
    // Renaming a repeated variable drops the equality the old template required.
    let repeated = single(op("/{x}/{x}"));
    assert!(widens(Some(&repeated), Some(&single(op("/{x}/{y}")))));
    // Narrowing one use of a repeated variable to a literal also drops equality.
    assert!(widens(Some(&repeated), Some(&single(op("/a/{x}")))));
    // A single-use variable may still be narrowed to a literal.
    assert!(!widens(Some(&plain), Some(&single(op("/files/report")))));
}

#[test]
fn widening_treats_input_limits_as_part_of_the_contract() {
    use nyxid_permissions::{ValueRule, values::InputRules};
    let open = single(op("/items/{id}"));
    let mut limited = open.clone();
    limited.operations[0].inputs = Some(InputRules {
        path: [(
            "id".to_string(),
            ValueRule::Exact {
                value: serde_json::json!("1"),
            },
        )]
        .into(),
        ..Default::default()
    });
    let mut other = limited.clone();
    other.operations[0].inputs.as_mut().unwrap().path.insert(
        "id".into(),
        ValueRule::Exact {
            value: serde_json::json!("2"),
        },
    );
    assert!(
        !widens(Some(&open), Some(&limited)),
        "adding limits narrows"
    );
    assert!(
        widens(Some(&limited), Some(&open)),
        "removing limits widens"
    );
    assert!(
        widens(Some(&limited), Some(&other)),
        "changing limits widens"
    );
    assert!(!widens(Some(&limited), Some(&limited)));
}

#[test]
fn input_limits_check_path_query_and_body_on_the_final_request() {
    use nyxid_permissions::{
        ValueRule,
        google::{JsonShape, QueryParameter},
        values::InputRules,
    };
    let mut read = op("/calendars/{calendar}/events");
    read.inputs = Some(InputRules {
        path: [(
            "calendar".to_string(),
            ValueRule::Exact {
                value: serde_json::json!("team"),
            },
        )]
        .into(),
        query: Some(
            [(
                "maxResults".to_string(),
                QueryParameter {
                    required: true,
                    rule: ValueRule::MaxInteger { value: 50 },
                },
            )]
            .into(),
        ),
        body: None,
    });
    let mut write = op("/notes");
    write.rule.method = "POST".into();
    write.endpoint_id = Some("write".into());
    write.inputs = Some(InputRules {
        body: Some(JsonShape::Object {
            properties: [("text".to_string(), JsonShape::String { max_length: 8 })].into(),
            required: vec!["text".into()],
        }),
        ..Default::default()
    });
    let scopes = OperationScopes::from([(
        "service".into(),
        AgentOperationScope {
            revision: 1,
            catalog_service_id: None,
            operations: vec![read, write],
        },
    )]);
    let check = |method: &str, path: &str, query: Option<&str>, body: &[u8]| {
        check_inputs(
            &scopes,
            "service",
            None,
            None,
            method,
            &CanonicalPath::from_mcp_literal(path).unwrap(),
            query,
            body,
            Some("application/json"),
        )
    };
    let events = "/calendars/team/events";
    assert!(check("GET", events, Some("maxResults=50"), b"").is_ok());
    assert!(check("GET", events, Some("maxResults=51"), b"").is_err());
    assert!(check("GET", events, None, b"").is_err(), "required filter");
    assert!(
        check("GET", events, Some("maxResults=5&pageToken=x"), b"").is_err(),
        "closed query"
    );
    assert!(check("GET", "/calendars/other/events", Some("maxResults=5"), b"").is_err());
    assert!(check("POST", "/notes", None, br#"{"text":"hi"}"#).is_ok());
    assert!(check("POST", "/notes", None, br#"{"text":"far too long"}"#).is_err());
    assert!(check("POST", "/notes", None, br#"{"text":"hi","x":1}"#).is_err());
    assert!(
        check("POST", "/notes", None, b"").is_err(),
        "streamed bodies refuse"
    );
    // Unscoped services are unaffected.
    assert!(
        check_inputs(
            &scopes,
            "other",
            None,
            None,
            "DELETE",
            &CanonicalPath::from_mcp_literal("/x").unwrap(),
            None,
            b"",
            None,
        )
        .is_ok()
    );
}

#[test]
fn contract_digest_distinguishes_compiled_contracts() {
    let a = single(op("/items/{id}"));
    let mut b = a.clone();
    b.revision = 7;
    assert_eq!(
        contract_digest(Some(&a)),
        contract_digest(Some(&b)),
        "revision excluded"
    );
    assert_ne!(
        contract_digest(Some(&a)),
        contract_digest(Some(&single(op("/items/one"))))
    );
    assert_ne!(contract_digest(Some(&a)), contract_digest(None));
}

async fn key_fixture(
    name: &str,
) -> (
    mongodb::Database,
    String,
    String,
    String,
    crate::models::api_key::ApiKey,
) {
    use crate::models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS},
        user::UserType,
    };
    let db = crate::test_utils::connect_transaction_test_database(name).await;
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(&owner, UserType::Person))
        .await
        .unwrap();
    crate::test_utils::set_agent_operation_scopes_enabled(&db, &owner, true).await;
    let service = crate::services::assistant_authority_tests::connected(
        &db,
        &owner,
        "scoped-key",
        "http://127.0.0.1:9",
    )
    .await;
    let now = chrono::Utc::now();
    let endpoint = uuid::Uuid::new_v4().to_string();
    db.collection(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(crate::models::service_endpoint::ServiceEndpoint {
            async_operation: None,
            target_id: None,
            id: endpoint.clone(),
            service_id: service.clone(),
            name: "list".into(),
            description: Some("List items".into()),
            method: "GET".into(),
            path: "/items".into(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: None,
            response: Default::default(),
            risk: None,
            supports_idempotency_key: false,
            is_active: true,
            operation_generation: 1,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let key = ApiKey {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: owner.clone(),
        name: "agent".into(),
        key_prefix: "nyxid_ag".into(),
        key_hash: "00".repeat(32),
        scopes: "proxy".into(),
        last_used_at: None,
        expires_at: None,
        is_active: true,
        created_at: now,
        rotation_predecessor_id: None,
        state_version: 1,
        updated_at: Some(now),
        description: None,
        allowed_service_ids: vec![service.clone()],
        allowed_platform_service_ids: Vec::new(),
        assistant_group_id: None,
        assistant_agent_owner_id: None,
        assistant_operation_scopes: Default::default(),
        allowed_node_ids: vec![],
        allow_all_services: false,
        allow_auto_connected_services: false,
        allow_all_nodes: true,
        rate_limit_per_second: None,
        rate_limit_burst: None,
        platform: Some("claude-code".into()),
        callback_url: None,
        purpose: ApiKeyPurpose::General,
        scheduled_write_enabled: false,
    };
    db.collection::<ApiKey>(KEYS)
        .insert_one(&key)
        .await
        .unwrap();
    (db, owner, service, endpoint, key)
}

fn selection(revision: i64, endpoint_ids: Vec<String>) -> OperationSelection {
    OperationSelection {
        expected_revision: revision,
        all_operations: false,
        endpoint_ids,
        rules: vec![],
        contract_digest: None,
        inputs: Default::default(),
    }
}

#[tokio::test]
async fn agent_key_operation_scopes_save_fence_revisions_and_reach_auth() {
    let (db, owner, service, endpoint, key) = key_fixture("agent_key_operation_scopes").await;
    let listed = key_options(&db, &owner, &key.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].all_operations);
    assert_eq!(listed[0].operations[0].endpoint_id, endpoint);

    let (saved, saved_revision) = set_key(
        &db,
        &owner,
        &key.id,
        &service,
        &selection(0, vec![endpoint.clone()]),
    )
    .await
    .unwrap();
    let scope = &saved.assistant_operation_scopes[&service];
    assert_eq!(scope.revision, 1);
    assert_eq!(scope.operations[0].rule.path_template, "/items");

    // A stale writer cannot overwrite the newer selection.
    let stale = set_key(&db, &owner, &key.id, &service, &selection(0, vec![])).await;
    assert!(matches!(stale, Err(crate::errors::AppError::Conflict(_))));

    // Authentication carries the key's scopes into every transport.
    let stored = crate::services::key_service::get_api_key(&db, &owner, &key.id)
        .await
        .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&db, &stored, None, None, None)
        .await
        .unwrap();
    assert!(auth.assistant_chat.is_none());
    let path = CanonicalPath::from_mcp_literal("/items").unwrap();
    assert!(
        authorize(
            &auth.assistant_operation_scopes,
            &service,
            None,
            None,
            "GET",
            &path,
            false,
            false
        )
        .is_ok()
    );
    assert!(
        authorize(
            &auth.assistant_operation_scopes,
            &service,
            None,
            None,
            "DELETE",
            &path,
            false,
            false
        )
        .is_err()
    );
    // Ordinary keys pass the conversation checks without a live conversation.
    assert!(
        !check_non_mcp_context(&db, &auth, true, &service, None, "GET", Some(&path))
            .await
            .unwrap()
    );

    // Returning to all operations removes the restriction.
    let all = OperationSelection {
        all_operations: true,
        ..selection(1, vec![])
    };
    let (cleared, cleared_revision) = set_key(&db, &owner, &key.id, &service, &all).await.unwrap();
    assert!(!cleared.assistant_operation_scopes.contains_key(&service));
    assert_eq!((saved_revision, cleared_revision), (1, 2));
    // The revision survives the return to all operations: a writer who last
    // saw revision 0 cannot overwrite the newer state.
    assert!(matches!(
        set_key(&db, &owner, &key.id, &service, &selection(0, vec![])).await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    let listed = key_options(&db, &owner, &key.id).await.unwrap();
    assert_eq!((listed[0].revision, listed[0].all_operations), (2, true));
    assert!(
        set_key(&db, &owner, &key.id, &service, &selection(2, vec![]))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn agent_key_operation_scopes_refuse_strangers_digests_and_other_key_kinds() {
    use crate::models::api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS};
    use mongodb::bson::doc;
    let (db, owner, service, endpoint, key) = key_fixture("agent_key_operation_scope_acl").await;
    let stranger = uuid::Uuid::new_v4().to_string();
    assert!(key_options(&db, &stranger, &key.id).await.is_err());
    assert!(
        set_key(&db, &stranger, &key.id, &service, &selection(0, vec![]))
            .await
            .is_err()
    );
    // A reviewed digest that no longer matches the compiled contract conflicts.
    let reviewed = OperationSelection {
        contract_digest: Some("stale".into()),
        ..selection(0, vec![endpoint.clone()])
    };
    assert!(matches!(
        set_key(&db, &owner, &key.id, &service, &reviewed).await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    // Unknown services and conversation or permission-bound keys are refused.
    assert!(
        set_key(&db, &owner, &key.id, "not-granted", &selection(0, vec![]))
            .await
            .is_err()
    );
    for update in [
        doc! {"$set": {"purpose": "permission_bound"}},
        doc! {"$set": {"purpose": "general", "assistant_agent_owner_id": &owner}},
    ] {
        db.collection::<ApiKey>(KEYS)
            .update_one(doc! {"_id": &key.id}, update)
            .await
            .unwrap();
        assert!(key_options(&db, &owner, &key.id).await.is_err());
    }
    let _ = ApiKeyPurpose::General;
}

#[tokio::test]
async fn unscoped_services_keep_guest_and_webhook_limits_on_raw_routes() {
    use crate::models::{
        assistant_agent::{AgentGrants, COLLECTION_NAME as AGENTS},
        service_endpoint::{COLLECTION_NAME as ENDPOINTS, EndpointRisk, ServiceEndpoint},
        trigger_schedule::ConfirmationPolicy,
    };
    use crate::services::{
        assistant_authority_tests::{connected, fixture},
        assistant_team_service as team,
    };
    use mongodb::bson::doc;
    let f = fixture("unscoped_guest_limits").await;
    let service = connected(&f.state.db, &f.owner, "unscoped", "http://127.0.0.1:9").await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<mongodb::bson::Document>(AGENTS)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {format!("guest_access.{service}"): "read"}},
        )
        .await
        .unwrap();
    // A stored contract may mark a POST as a read; nothing else widens guests.
    let now = chrono::Utc::now();
    f.state
        .db
        .collection(ENDPOINTS)
        .insert_one(ServiceEndpoint {
            async_operation: None,
            target_id: None,
            id: uuid::Uuid::new_v4().to_string(),
            service_id: service.clone(),
            name: "search".into(),
            description: None,
            method: "POST".into(),
            path: "/search".into(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: None,
            response: Default::default(),
            risk: Some(EndpointRisk::Read),
            supports_idempotency_key: false,
            is_active: true,
            operation_generation: 1,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let as_chat = |chat: crate::services::assistant_acknowledgement_service::ChatAuthority| {
        let mut auth = f.auth.clone();
        auth.assistant_chat = Some(std::sync::Arc::new(chat));
        auth
    };
    let guest = as_chat(
        crate::services::assistant_acknowledgement_service::ChatAuthority {
            guest: true,
            ..f.chat.clone()
        },
    );
    let check = |auth: &crate::mw::auth::AuthUser, method: &'static str, path: &'static str| {
        let db = f.state.db.clone();
        let auth = auth.clone();
        let service = service.clone();
        async move {
            let path = CanonicalPath::from_mcp_literal(path).unwrap();
            check_non_mcp_context(&db, &auth, false, &service, None, method, Some(&path)).await
        }
    };
    assert!(
        check(&guest, "GET", "/items/1").await.unwrap(),
        "guest flag returned"
    );
    assert!(check(&guest, "DELETE", "/items/1").await.is_err());
    assert!(
        check(&guest, "POST", "/items").await.is_err(),
        "a POST is not a read"
    );
    assert!(
        check(&guest, "POST", "/search").await.is_ok(),
        "contract read"
    );

    let webhook = as_chat(
        crate::services::assistant_acknowledgement_service::ChatAuthority {
            guest: false,
            confirmation_policy: Some(ConfirmationPolicy::Changes),
            ..f.chat.clone()
        },
    );
    assert!(!check(&webhook, "GET", "/items/1").await.unwrap());
    assert!(check(&webhook, "POST", "/items").await.is_err());

    // Ordinary keys carry no turn authority and acquire no extra checks.
    let mut ordinary = f.auth.clone();
    ordinary.assistant_chat = None;
    ordinary.assistant_agent_owner_id = None;
    assert!(!check(&ordinary, "DELETE", "/items/1").await.unwrap());
    f.state.db.drop().await.unwrap();
}

#[test]
fn widening_cards_spell_out_value_limit_changes() {
    use nyxid_permissions::{ValueRule, values::InputRules};
    let limits = |max: u64| InputRules {
        query: Some(
            [(
                "maxResults".to_string(),
                nyxid_permissions::google::QueryParameter {
                    required: true,
                    rule: ValueRule::MaxInteger { value: max },
                },
            )]
            .into(),
        ),
        ..Default::default()
    };
    let current = ServiceOptions {
        service_id: "svc".into(),
        service_slug: "calendar".into(),
        service_name: "Calendar".into(),
        revision: 3,
        all_operations: false,
        allows_explicit_rules: false,
        endpoint_ids: vec!["list".into()],
        rules: vec![],
        inputs: [("list".to_string(), limits(10))].into(),
        operations: vec![OperationOption {
            endpoint_id: "list".into(),
            method: "GET".into(),
            path: "/events".into(),
            summary: None,
            read_only: true,
            changes_existing: false,
        }],
    };
    let relaxed = OperationSelection {
        inputs: [("list".to_string(), limits(100_000))].into(),
        ..selection(3, vec!["list".into()])
    };
    let text = limit_changes(&current, &relaxed);
    assert!(text.contains("GET /events: limits changed to"), "{text}");
    assert!(text.contains("100000"), "{text}");
    assert!(limit_changes(&current, &selection(3, vec!["list".into()])).contains("limits removed"));
    let same = OperationSelection {
        inputs: current.inputs.clone(),
        ..selection(3, vec!["list".into()])
    };
    assert!(limit_changes(&current, &same).is_empty());
}
