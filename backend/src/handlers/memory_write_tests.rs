use super::*;
use crate::{
    errors::{AppError, memory_write::MemoryWriteReason as Reason},
    models::assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS},
    services::{assistant_authority_tests::orchestrator_fixture, assistant_team_service as team},
};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};

async fn call(state: &AppState, auth: &McpAuthContext, args: Value) -> Value {
    let response = Box::pin(handle_tools_call(
        state,
        auth,
        None,
        &JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: Some(json!({"name":"nyxid__remember", "arguments":args})),
        },
        false,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    ))
    .await;
    let bytes = axum::body::to_bytes(response.into_body(), 65_536)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn result(value: &Value) -> Value {
    serde_json::from_str(value["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

async fn assert_reason(state: &AppState, auth: &McpAuthContext, args: Value, reason: Reason) {
    let response = call(state, auth, args).await;
    assert_eq!(response["result"]["isError"], true, "{response}");
    let value = result(&response);
    assert_eq!(value["details"]["reason"], json!(reason));
    assert!(!value.to_string().contains("DO_NOT_DISCLOSE"));
    assert!(value.to_string().len() < 1024);
    assert_eq!(value["details"]["category"], reason.category());
    if reason == Reason::MemoryUnauthorized {
        assert_eq!(value["error_code"], 1003, "keep the scoped not-found code");
    }
    assert!(!value["message"].as_str().unwrap().is_empty());
    let chat = auth.chat.as_ref().unwrap();
    let row =
        crate::services::assistant_nyxagent::get(&state.db, &chat.user_id, &chat.conversation_id)
            .await
            .unwrap();
    let activities = row.active_turn.unwrap().activities;
    let last = activities.last().unwrap();
    assert_eq!(last.status, "error");
    assert_eq!(
        last.failure.as_ref().unwrap().reason.as_deref(),
        json!(reason).as_str()
    );
}

#[tokio::test]
async fn remember_safe_reasons_reach_mcp_and_persisted_activity() {
    let f = orchestrator_fixture("remember_safe_reasons").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    for (args, reason) in [
        (json!({"text":"x".repeat(501)}), Reason::NoteTooLong),
        (json!({"text":" \n\t "}), Reason::NoteEmpty),
        (json!({"text":""}), Reason::NoteEmpty),
        (json!({"text":123}), Reason::InvalidNote),
        (
            json!({"text":"private token nyxid_ag_DO_NOT_DISCLOSE"}),
            Reason::SecretDetected,
        ),
        (
            json!({"text":"A fact", "replace_id":"absent"}),
            Reason::ReplaceTargetNotFound,
        ),
        (
            json!({"text":"A fact", "agent":uuid::Uuid::new_v4().to_string()}),
            Reason::MemoryUnauthorized,
        ),
    ] {
        assert_reason(&f.state, &auth, args, reason).await;
    }
    let rows = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":"assistant_team_tool_call"})
        .await
        .unwrap();
    use futures::TryStreamExt;
    let audit = rows.try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(audit.len(), 7);
    assert!(!format!("{audit:?}").contains("DO_NOT_DISCLOSE"));
    // Later valid writes commit without relabelling the rejected calls as saved.
    let saved = call(&f.state, &auth, json!({"text":"Accept risk-averse, task-oriented business plans; reserve cakes 48 hours ahead."})).await;
    assert_eq!(saved["result"]["isError"], false, "{saved}");
    assert!(result(&saved)["remembered"].is_string());
    let row = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let activities = row.active_turn.unwrap().activities;
    assert_eq!(activities.iter().filter(|a| a.status == "error").count(), 7);
    assert_eq!(activities.last().unwrap().status, "completed");
    assert_eq!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .memory
            .len(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remember_capacity_and_destroyed_conflicts_are_distinct() {
    let f = orchestrator_fixture("remember_capacity").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    for i in 0..50 {
        team::remember(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &format!("Fact {i}"),
            None,
        )
        .await
        .unwrap();
    }
    assert_reason(
        &f.state,
        &auth,
        json!({"text":"Fact 51"}),
        Reason::MemoryFull,
    )
    .await;
    let row = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(row.memory.len(), 50);
    f.state
        .db
        .collection::<bson::Document>(AGENTS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    for replace in [None, Some(row.memory[0].id.as_str())] {
        let error = team::remember(&f.state.db, &f.owner, &row.id, "Not saved", replace)
            .await
            .unwrap_err();
        assert_eq!(Reason::from_error(&error), Reason::MemoryConflict);
    }
    assert_eq!(
        team::agent(&f.state.db, &f.owner, &row.id)
            .await
            .unwrap()
            .memory[0]
            .text,
        "Fact 0"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remember_storage_rejection_is_safe_and_later_save_does_not_erase_failure() {
    let f = orchestrator_fixture("remember_storage_rejection").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    // Database-local rejection, with no global failpoint or interference with other tests.
    f.state
        .db
        .run_command(
            doc! {"collMod":AGENTS,"validator":{"memory":{"$size":0}},"validationLevel":"strict"},
        )
        .await
        .unwrap();
    assert_reason(
        &f.state,
        &auth,
        json!({"text":"A durable business preference"}),
        Reason::MemoryStorageError,
    )
    .await;
    assert!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .memory
            .is_empty()
    );
    f.state
        .db
        .run_command(doc! {"collMod":AGENTS,"validator":{}})
        .await
        .unwrap();
    let saved = call(
        &f.state,
        &auth,
        json!({"text":"A durable business preference"}),
    )
    .await;
    assert_eq!(saved["result"]["isError"], false);
    assert_eq!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .memory
            .len(),
        1
    );
    let internal = crate::services::assistant_account_tools::memory_error_result_for(
        AppError::Internal("sensitive internals".into()),
        Some(&f.chat),
        "https://example.com",
    );
    assert_eq!(internal.value["details"]["reason"], "memory_internal_error");
    assert_eq!(internal.value["error_code"], 1006);
    assert!(!internal.value.to_string().contains("sensitive internals"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remember_concurrent_calls_in_one_turn_commit_each_note_once() {
    let f = orchestrator_fixture("remember_parallel").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    let results = futures::future::join_all((0..8).map(|i| {
        call(
            &f.state,
            &auth,
            json!({"text":format!("Concurrent fact {i}")}),
        )
    }))
    .await;
    let mut ids = std::collections::HashSet::new();
    for value in results {
        assert_eq!(value["result"]["isError"], false, "{value}");
        assert!(ids.insert(result(&value)["remembered"].as_str().unwrap().to_owned()));
    }
    let agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(agent.memory.len(), 8);
    let row = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert!(
        row.active_turn
            .unwrap()
            .activities
            .iter()
            .all(|a| a.status == "completed")
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remember_concurrent_replacements_return_their_own_committed_snapshot() {
    let f = orchestrator_fixture("remember_replace_snapshot").await;
    let original = team::remember(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        "Initial fact",
        None,
    )
    .await
    .unwrap();
    let writes = (0..12).map(|i| {
        let f = &f;
        let original = &original;
        async move {
            let text = format!("Replacement {i}");
            let note = team::remember(
                &f.state.db,
                &f.owner,
                &f.chat.agent_id,
                &text,
                Some(&original.id),
            )
            .await
            .unwrap();
            assert_eq!(note.text, text);
            assert_eq!(
                note.created_at.timestamp_millis(),
                original.created_at.timestamp_millis()
            );
        }
    });
    futures::future::join_all(writes).await;
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAgent>(AGENTS)
            .find_one(doc! {"_id":&f.chat.agent_id})
            .await
            .unwrap()
            .unwrap()
            .memory
            .len(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[test]
fn remember_business_text_is_not_a_secret_but_credential_shapes_stay_blocked() {
    for text in [
        "Task-oriented service for risk-averse merchants",
        "Cake prices: $48. Deposits: 50%. Pick up before 6pm.",
        "菜单更新：每周一；提前两天预订。",
    ] {
        assert!(team::validate_memory_text(text).is_ok());
    }
    for text in [
        "sk-abcdefghijklmnopqrstuvwx",
        "password: hidden",
        "github_pat_hidden",
        "-----BEGIN PRIVATE KEY-----",
    ] {
        assert!(matches!(
            team::validate_memory_text(text),
            Err(AppError::MemoryWrite(Reason::SecretDetected))
        ));
    }
}

#[tokio::test]
async fn remember_concurrent_capacity_boundary_never_exceeds_fifty() {
    let f = orchestrator_fixture("remember_capacity_race").await;
    for i in 0..49 {
        team::remember(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &format!("Fact {i}"),
            None,
        )
        .await
        .unwrap();
    }
    let (a, b) = tokio::join!(
        team::remember(&f.state.db, &f.owner, &f.chat.agent_id, "Candidate A", None),
        team::remember(&f.state.db, &f.owner, &f.chat.agent_id, "Candidate B", None),
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let error = a.err().or(b.err()).unwrap();
    assert_eq!(Reason::from_error(&error), Reason::MemoryFull);
    assert_eq!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .memory
            .len(),
        50
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remember_wrapped_mcp_call_keeps_the_safe_failure_reason() {
    let f = orchestrator_fixture("remember_wrapped_mcp").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    let response = Box::pin(handle_tools_call(
        &f.state, &auth, None,
        &JsonRpcRequest { jsonrpc: JSONRPC_VERSION.into(), id: Some(json!(1)), method: "tools/call".into(),
            params: Some(json!({"name":"nyx__call_tool", "arguments":{"tool_name":"nyxid__remember", "arguments_json":json!({"text":"x".repeat(501)}).to_string()}})) },
        false, crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    )).await;
    assert_eq!(
        tool_failure(&response).unwrap().reason.as_deref(),
        Some("note_too_long")
    );
    let row = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let activity = row.active_turn.unwrap().activities.pop().unwrap();
    assert_eq!(activity.label, "nyxid__remember");
    assert_eq!(activity.status, "error");
    assert_eq!(
        activity.failure.unwrap().reason.as_deref(),
        Some("note_too_long")
    );
    f.state.db.drop().await.unwrap();
}

#[test]
fn remember_failure_metadata_is_additive_and_only_native_codes_are_trusted() {
    use crate::models::assistant_conversation::ToolFailure;
    let legacy: ToolFailure = serde_json::from_value(json!({"kind":"tool_error"})).unwrap();
    assert!(legacy.reason.is_none());
    assert_eq!(
        serde_json::to_value(legacy).unwrap(),
        json!({"kind":"tool_error"})
    );
    for (name, reason) in [
        ("downstream__remember", "note_too_long"),
        ("nyxid__remember", "untrusted diagnostic"),
    ] {
        let response =
            native_team_tool_result(None, name, &json!({"details":{"reason":reason}}), true);
        assert!(tool_failure(&response).unwrap().reason.is_none());
    }
}
