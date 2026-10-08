//! NyxBot's production profile is wire v1 with independent authority v2 support.
use super::*;
use crate::models::assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS};
use crate::services::machine_access_service as access;

async fn production() -> Fixture {
    let f = Box::pin(orchestrator_fixture("machine_discovery_production")).await;
    let node = Box::pin(node(&f, &f.owner)).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &node.id},
            doc! {"$set": {
                "name": "work-macbook-air", "agent_version": "0.66.0",
                "machine.version": 1, "machine.authority_versions": [2],
                "machine.browser": bson::Bson::Null, "machine.separated": {"available": true}
            }},
        )
        .await
        .unwrap();
    let agent = crate::services::org_agent_service::chat_agent(&f.state.db, &f.chat)
        .await
        .unwrap();
    let policy = agent.machine_access.as_ref().unwrap();
    assert_eq!(policy.version, 2);
    assert_eq!(policy.revision, 1);
    let assignment = &policy.assignments[&node.id];
    assert!(assignment.legacy && assignment.capabilities.shell && assignment.capabilities.files);
    assert_eq!(assignment.mode, "shared_legacy");
    assert_eq!(assignment.revision, 1);
    assert!(agent.machine_node_ids.is_empty());
    f
}

#[tokio::test]
async fn machine_discovery_production_policy_exposes_execution_and_files() {
    let f = Box::pin(production()).await;
    let auth = authenticated_machine_chat(&f).await;
    let tools = Box::pin(access::definitions(
        &f.state.db,
        auth.chat.as_ref().unwrap(),
    ))
    .await
    .unwrap();
    for name in [
        "nyx__machine_exec",
        "nyx__machine_read_file",
        "nyx__machine_write_file",
        "nyx__machine_edit_file",
    ] {
        assert!(tools.iter().any(|tool| tool.name == name), "missing {name}");
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_discovery_production_search_reaches_the_model() {
    let f = Box::pin(production()).await;
    let auth = authenticated_machine_chat(&f).await;
    let mut within_budget = true;
    for (query, expected) in [
        ("run python code on my machine", "nyx__machine_exec"),
        ("execute command", "nyx__machine_exec"),
        ("write a file", "nyx__machine_write_file"),
        ("edit file", "nyx__machine_edit_file"),
        ("read file", "nyx__machine_read_file"),
    ] {
        let response = Box::pin(handle_meta_search(
            &f.state,
            &auth,
            None,
            &json!({"query": query}),
            Some(json!(1)),
            false,
        ))
        .await;
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let envelope: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(envelope["result"]["isError"], false);
        let text = envelope["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(text).unwrap();
        assert!(
            result["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == expected),
            "{query}"
        );
        let position = text.find(expected).unwrap();
        eprintln!(
            "{query}: chars={}, expected_tool_offset={position}",
            text.chars().count()
        );
        within_budget &= envelope["result"].to_string().len() <= 10_000;
        assert_eq!(result["matches"][0]["name"], expected, "{query}");
        assert!(
            result["matches"][0]["inputSchema"].is_object(),
            "keep complete schemas for these tools"
        );
        within_budget &= position < 10_000;
    }
    f.state.db.drop().await.unwrap();
    assert!(
        within_budget,
        "machine search results must survive NyxAgent's 10,000-character tool budget"
    );
}

#[tokio::test]
async fn machine_discovery_failures_warn_without_content_and_keep_other_tools() {
    use tracing::instrument::WithSubscriber;
    let f = Box::pin(production()).await;
    let auth = authenticated_machine_chat(&f).await;
    f.state
        .db
        .collection::<bson::Document>(AGENTS)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"machine_access.version": 99}},
        )
        .await
        .unwrap();
    let capture = tempfile::NamedTempFile::new().unwrap();
    let writer = capture.reopen().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.try_clone().unwrap())
        .finish();
    async {
        let response = Box::pin(handle_tools_list(
            &f.state,
            &auth,
            None,
            &JsonRpcRequest {
                jsonrpc: JSONRPC_VERSION.into(),
                id: Some(json!(1)),
                method: "tools/list".into(),
                params: None,
            },
        ))
        .await;
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let envelope: Value = serde_json::from_slice(&bytes).unwrap();
        let tools = envelope["result"]["tools"].as_array().unwrap();
        assert!(!tools.iter().any(|tool| tool["name"] == "nyx__machine_exec"));
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "nyx__attachment_read")
        );
        assert!(tools.iter().any(|tool| tool["name"] == "nyx__search_tools"));
        let result = machine_mcp_call(
            &f,
            &auth,
            "nyx__search_tools",
            json!({"query": "read attachment query-private-sentinel"}),
        )
        .await;
        assert!(
            result["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "nyx__attachment_read")
        );
    }
    .with_subscriber(subscriber)
    .await;
    let logs = std::fs::read_to_string(capture.path()).unwrap();
    let warnings: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("Machine tool discovery unavailable"))
        .collect();
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings
            .iter()
            .all(|line| line.contains("machine_permission_revoked"))
    );
    for private in [
        &f.owner,
        &f.row.id,
        &f.chat.api_key_id,
        "query-private-sentinel",
    ] {
        assert!(!logs.contains(private));
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_machine_guidance_uses_loaded_nyxbot_assignments_and_keeps_specialists() {
    use crate::models::machine_access::{Assignment, Policy};
    use crate::models::{assistant_agent::AgentKind, assistant_conversation::AgentRole};
    use crate::services::assistant_instruction_context::Prepared;
    let f = Box::pin(production()).await;
    let mut agent = crate::services::org_agent_service::chat_agent(&f.state.db, &f.chat)
        .await
        .unwrap();
    let mut row = f.row.clone();
    row.nyxagent_session_id = None;
    let key = uuid::Uuid::new_v4();
    let prepare = |row: &crate::models::assistant_conversation::AssistantConversation,
                   agent: &AssistantAgent| {
        Prepared::new(key.as_bytes(), row, Some(agent), &[]).unwrap()
    };
    let first = prepare(&row, &agent);
    assert!(
        first
            .instructions(&[], false)
            .contains(crate::services::machine_tools::USE_INSTRUCTIONS)
    );
    assert!(
        first
            .instructions(&[], false)
            .contains(crate::services::machine_tools::SETUP_INSTRUCTIONS)
    );
    let mut binding = first.binding.clone();
    binding.session_id = uuid::Uuid::new_v4().to_string();
    row.nyxagent_session_id = Some(binding.session_id.clone());
    row.nyxagent_instruction_binding = Some(binding);
    assert!(!prepare(&row, &agent).reset);
    let assigned = agent.machine_access.clone();
    for policy in [
        None,
        Some(Box::default()),
        Some(Box::new(Policy {
            assignments: [(uuid::Uuid::new_v4().to_string(), Assignment::default())].into(),
            ..Default::default()
        })),
    ] {
        agent.machine_access = policy;
        let prepared = prepare(&row, &agent);
        assert!(
            prepared.reset,
            "removing use guidance must refresh the stable binding"
        );
        assert!(
            !prepared
                .instructions(&[], false)
                .contains(crate::services::machine_tools::USE_INSTRUCTIONS)
        );
        assert!(
            prepared
                .instructions(&[], false)
                .contains(crate::services::machine_tools::SETUP_INSTRUCTIONS)
        );
    }
    agent.machine_access = assigned;
    agent.machine_access.as_mut().unwrap().version = 99;
    assert!(
        !prepare(&row, &agent)
            .instructions(&[], false)
            .contains(crate::services::machine_tools::USE_INSTRUCTIONS)
    );
    agent.machine_access.as_mut().unwrap().version = 2;
    row.guest_turn = true;
    let guest = prepare(&row, &agent).instructions(&[], false);
    assert!(!guest.contains(crate::services::machine_tools::USE_INSTRUCTIONS));
    assert!(!guest.contains(crate::services::machine_tools::SETUP_INSTRUCTIONS));
    row.guest_turn = false;
    row.role = AgentRole::Subagent;
    agent.kind = AgentKind::Specialist;
    // A policy alone never gives specialists the membership needed for guidance.
    assert!(
        !prepare(&row, &agent)
            .instructions(&[], false)
            .contains(crate::services::machine_tools::USE_INSTRUCTIONS)
    );
    agent
        .machine_node_ids
        .push(uuid::Uuid::new_v4().to_string());
    agent.machine_access = None; // pre-policy specialist rule is unchanged
    let specialist = prepare(&row, &agent).instructions(&[], false);
    assert!(specialist.contains(crate::services::machine_tools::USE_INSTRUCTIONS));
    assert!(!specialist.contains(crate::services::machine_tools::SETUP_INSTRUCTIONS));
    f.state.db.drop().await.unwrap();
}

#[test]
fn machine_discovery_search_budget_handles_escaped_text_and_one_oversized_schema() {
    let name = "nyx__machine_exec";
    let response = json!({"matches":[
        {"name":name, "description":"Run code", "inputSchema":{"description":"\\\"雪".repeat(10000)}},
        {"name":"lower_rank", "description":"Other tool", "inputSchema":{}}
    ], "count":2});
    let text = bounded_chat_search_result(response);
    let wire = json!({"content":[{"type":"text","text":&text}], "isError":false});
    assert!(wire.to_string().len() <= 10_000);
    let result: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result["matches"][0]["name"], name);
    assert_eq!(result["matches"][0]["input_schema_omitted"], true);
    assert!(result["matches"][0].get("inputSchema").is_none());
    assert_eq!(result["truncated"], true);
    assert_eq!(result["count"], 1);
    // Bounds include JSON escaping, even for malformed legacy metadata.
    let text = bounded_chat_search_result(json!({"matches": [{
        "name": "\0".repeat(1024), "description": "\0".repeat(512), "inputSchema": {}
    }], "count": 1}));
    let wire = json!({"content":[{"type":"text","text":text}], "isError":false});
    assert!(wire.to_string().len() <= 10_000);
}

#[test]
fn machine_discovery_natural_queries_share_ranking_without_filler_matches() {
    for (query, expected) in [
        ("run python code on my machine", "nyx__machine_exec"),
        ("python", "nyx__machine_exec"),
        ("execute command", "nyx__machine_exec"),
        ("write a file", "nyx__machine_write_file"),
        ("edit file", "nyx__machine_edit_file"),
        ("read file", "nyx__machine_read_file"),
    ] {
        let matcher = mcp_service::ToolSearch::new(query);
        let mut ranked: Vec<_> = crate::services::machine_tools::definitions()
            .into_iter()
            .filter_map(|tool| {
                matcher
                    .rank(&tool.name, &tool.description)
                    .map(|rank| (rank, tool.name))
            })
            .collect();
        ranked.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
        assert_eq!(ranked[0].1, expected, "{query}");
    }
    assert!(
        mcp_service::ToolSearch::new("write a file")
            .rank("nyxid__get_agent", "Manage an agent")
            .is_none()
    );
}

/// #1800: a browserless machine is usable in its configured shared mode.
/// Exercise authentication, native/universal MCP dispatch, live assignment
/// checks, signed node transport and mode diagnostics for both agent roles.
#[tokio::test]
async fn machine_discovery_headless_shared_exec_and_file_tools_never_downgrade_separated() {
    use crate::models::machine_access::{Assignment, Policy};
    use nyxid_machine::{Confirmation, Operation, authority::Capabilities, context::Support};
    for specialist in [false, true] {
        let f = if specialist {
            Box::pin(fixture("machine_discovery_headless_specialist")).await
        } else {
            Box::pin(orchestrator_fixture("machine_discovery_headless_nyxbot")).await
        };
        let mut node = Box::pin(node(&f, &f.owner)).await;
        node.machine_confirm = Confirmation::None;
        let profile = node.machine.as_mut().unwrap();
        profile.authority_versions = vec![2];
        profile.computer = false;
        profile.browser = Some(false);
        profile.browser_tools = false;
        profile.separated = Some(Support {
            available: false,
            landlock_abi: Some(8),
            reason: Some("separated_requires_managed_browser".into()),
        });
        f.state
            .db
            .collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &node.id},
                doc! {"$set": {
                    "machine": bson::to_bson(&node.machine).unwrap(), "machine_confirm": "none"
                }},
            )
            .await
            .unwrap();
        let policy = Policy {
            revision: 3,
            assignments: [(
                node.id.clone(),
                Assignment {
                    revision: 3,
                    legacy: true,
                    capabilities: Capabilities {
                        shell: true,
                        files: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let mut fields = doc! {"machine_access": bson::to_bson(&policy).unwrap()};
        if specialist {
            fields.insert("machine_node_ids", vec![node.id.clone()]);
        }
        f.state
            .db
            .collection::<bson::Document>(AGENTS)
            .update_one(doc! {"_id": &f.chat.agent_id}, doc! {"$set": fields})
            .await
            .unwrap();
        let auth = authenticated_machine_chat(&f).await;
        let agent = crate::services::org_agent_service::chat_agent(
            &f.state.db,
            auth.chat.as_ref().unwrap(),
        )
        .await
        .unwrap();
        let guidance = crate::services::assistant_nyxagent::base_prompt(&f.row, Some(&agent));
        assert!(guidance.contains(
            "configured as shared_legacy runs permitted commands and file operations normally"
        ));
        assert!(
            guidance.contains("Only an assignment configured as separated must refuse execution")
        );
        assert!(
            !guidance.contains("Never fall back to shared work when separation is unavailable.")
        );
        let diagnostics =
            machine_mcp_call(&f, &auth, "nyxid__machine_capabilities", json!({})).await;
        assert_eq!(diagnostics["machines"][0]["mode"], "shared_legacy");
        assert_eq!(diagnostics["machines"][0]["revision"], 3);
        assert!(
            diagnostics["machines"][0]["execution_note"]
                .as_str()
                .unwrap()
                .contains("shared mode, as configured")
        );
        assert!(
            diagnostics["machines"][0]["separated_setup_note"]
                .as_str()
                .unwrap()
                .contains("owner action card")
        );
        let listing = machine_mcp_call(&f, &auth, "nyx__machine_list", json!({})).await;
        assert!(
            listing["machines"][0]["execution_note"]
                .as_str()
                .unwrap()
                .contains("shared mode, as configured")
        );
        let tools = Box::pin(access::definitions(
            &f.state.db,
            auth.chat.as_ref().unwrap(),
        ))
        .await
        .unwrap();
        assert!(tools.iter().any(|t| t.name == "nyx__machine_exec"));
        assert!(tools.iter().any(|t| t.name == "nyx__machine_write_file"));
        assert!(!tools.iter().any(|t| t.name == "nyx__machine_browser"));
        assert!(
            tools
                .iter()
                .find(|t| t.name == "nyx__machine_exec")
                .unwrap()
                .description
                .contains("Configured shared_legacy works even when separation is unavailable")
        );
        let (task, mut requests) = peer(
            &f,
            &node,
            json!({"exit_code":0,"status":"finished","bytes":6}),
        )
        .await;
        let exec = machine_mcp_call(&f, &auth, "nyx__call_tool", json!({
            "tool_name":"nyx__machine_exec",
            "arguments_json":json!({"machine":node.id,"command":"true","services":[]}).to_string()
        })).await;
        assert_eq!(exec["exit_code"], 0, "{exec}");
        let write = machine_mcp_call(
            &f,
            &auth,
            "nyx__machine_write_file",
            json!({
                "machine":node.id,"path":"task/hello.txt","content":"hello\n","mode":"create"
            }),
        )
        .await;
        assert_eq!(write["bytes"], 6, "{write}");
        for operation in [Operation::Exec, Operation::WriteFile] {
            let request = requests.recv().await.unwrap();
            assert_eq!(request.operation, operation);
            assert_eq!(request.version, 2);
            match operation {
                Operation::Exec => assert_eq!(request.parameters["command"], "true"),
                Operation::WriteFile => {
                    assert_eq!(request.parameters["path"], "task/hello.txt");
                    assert_eq!(request.parameters["content"], "hello\n");
                    assert_eq!(request.parameters["mode"], "create");
                }
                _ => unreachable!(),
            }
            let authority = request.authority.unwrap();
            assert_eq!(authority.mode, "shared_legacy");
            assert_eq!(authority.revision, 3);
        }
        // The same headless node must refuse a stored separated assignment,
        // including read/edit paths; availability is never permission to downgrade.
        let path = format!("machine_access.assignments.{}", node.id);
        f.state
            .db
            .collection::<bson::Document>(AGENTS)
            .update_one(
                doc! {"_id": &agent.id},
                doc! {"$set": {
                    format!("{path}.mode"): "separated", format!("{path}.legacy"): false
                }},
            )
            .await
            .unwrap();
        for (tool, args) in [
            ("nyx__machine_exec", json!({"command":"true"})),
            (
                "nyx__machine_write_file",
                json!({"path":"task/hello.txt","content":"hello\n","mode":"create"}),
            ),
            ("nyx__machine_read_file", json!({"path":"task/hello.txt"})),
            (
                "nyx__machine_edit_file",
                json!({"path":"task/hello.txt","old_string":"hello","new_string":"hi"}),
            ),
        ] {
            let mut args = args;
            args["machine"] = json!(node.id);
            let result = machine_mcp_call(&f, &auth, tool, args).await;
            assert_eq!(result["error_code"], 12419, "{tool}: {result}");
        }
        assert!(requests.try_recv().is_err());
        let diagnostics =
            machine_mcp_call(&f, &auth, "nyxid__machine_capabilities", json!({})).await;
        assert!(
            diagnostics["machines"][0]["execution_note"]
                .as_str()
                .unwrap()
                .contains("Execution is refused")
        );
        task.abort();
        f.state.db.drop().await.unwrap();
    }
}
