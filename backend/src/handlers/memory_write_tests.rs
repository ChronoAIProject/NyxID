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

#[test]
fn remember_rejection_reference_does_not_count_as_continuation_progress() {
    let response = |reason, id| {
        native_team_tool_result(
            None,
            "nyxid__remember",
            &json!({"details":{"reason":reason,"failed_activity_id":id}}),
            true,
        )
    };
    let first = response("note_too_long", "one");
    let repeated = response("note_too_long", "two");
    let different = response("note_empty", "three");
    let digest = |response: &Response| {
        response
            .extensions()
            .get::<ToolResultDigest>()
            .unwrap()
            .0
            .clone()
    };
    assert_eq!(digest(&first), digest(&repeated));
    assert_ne!(digest(&first), digest(&different));
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
async fn remember_recovery_links_only_its_parts_and_survives_settlement() {
    use crate::services::assistant_nyxagent as engine;

    let f = orchestrator_fixture("remember_recovery_parts").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    let failed = result(&call(&f.state, &auth, json!({"text": "x".repeat(501)})).await);
    let original = failed["details"]["failed_activity_id"].as_str().unwrap();
    let part = |text: &str, part: u8, total: u8| json!({"text": text, "recovery": {"failed_activity_id": original, "part": part, "total": total}});
    let unrelated = result(&call(&f.state, &auth, json!({"text":"Independent fact"})).await);
    assert!(unrelated["remembered"].is_string());
    assert!(unrelated["recovery"].is_null());

    let (a, b) = tokio::join!(
        call(&f.state, &auth, part("First fact", 1, 2)),
        call(&f.state, &auth, part("First fact", 1, 2)),
    );
    let (a, b) = (result(&a), result(&b));
    assert!(a["remembered"].is_string() ^ b["remembered"].is_string());
    let (saved, duplicate) = if a["remembered"].is_string() {
        (a, b)
    } else {
        (b, a)
    };
    assert_eq!(saved["recovery"]["committed"], true);
    assert_eq!(duplicate["error"], "memory_recovery_unavailable");
    let wrong_total = result(&call(&f.state, &auth, part("Extra fact", 2, 3)).await);
    assert_eq!(wrong_total["error"], "memory_recovery_unavailable");
    let mut wrong_agent = part("Different agent", 2, 2);
    wrong_agent["agent"] = json!(uuid::Uuid::new_v4().to_string());
    assert_eq!(
        result(&call(&f.state, &auth, wrong_agent).await)["error"],
        "memory_recovery_unavailable"
    );

    let rejected = result(&call(&f.state, &auth, part(" ", 2, 2)).await);
    assert_eq!(rejected["details"]["reason"], "note_empty");
    assert!(
        rejected["recovery"].is_null(),
        "validation runs before reserving a part"
    );
    let saved = result(&call(&f.state, &auth, part("Second fact", 2, 2)).await);
    assert_eq!(saved["recovery"]["committed"], true);

    let message_id = uuid::Uuid::new_v4().to_string();
    engine::finish_turn(
        &f.state.db,
        &f.row,
        &f.row.credential_api_key_id,
        &message_id,
        &engine::TurnResult {
            text: "Both facts saved".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let (_, messages) = engine::history_page(&f.state.db, &f.owner, &f.row.id, 10, None)
        .await
        .unwrap();
    let message = messages
        .into_iter()
        .find(|message| message.id == message_id)
        .unwrap();
    let origin = message
        .activities
        .iter()
        .find(|activity| activity.id == original)
        .unwrap();
    assert_eq!(
        origin.failure.as_ref().unwrap().reason.as_deref(),
        Some("note_too_long")
    );
    let receipts: Vec<_> = message
        .activities
        .into_iter()
        .filter_map(|activity| {
            let wire = serde_json::to_value(
                crate::handlers::assistant_nyxagent::ActivityResponse::from(activity),
            )
            .unwrap();
            assert!(wire.get("memory_recovery_target").is_none());
            wire.get("memory_recovery").cloned()
        })
        .collect();
    assert_eq!(
        receipts,
        [
            json!({"failed_activity_id":original,"part":1,"total":2,"committed":true}),
            json!({"failed_activity_id":original,"part":2,"total":2,"committed":true}),
        ]
    );
    assert_eq!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .memory
            .len(),
        3
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remember_recovery_claims_only_after_confirmation() {
    use crate::{
        models::{assistant_settings::AccessMode, trigger_schedule::ConfirmationPolicy},
        services::{
            assistant_acknowledgement_service as acks, assistant_nyxagent as engine,
            assistant_settings_service,
        },
    };

    for continuation in [false, true] {
        let f = orchestrator_fixture("remember_recovery_approval").await;
        let mut auth = if continuation {
            super::machine_mcp_tests::trigger_auth(&f, Some(ConfirmationPolicy::Changes)).await
        } else {
            assistant_settings_service::update(
                &f.state.db,
                &f.owner,
                assistant_settings_service::Update {
                    access_mode: Some(AccessMode::AskEveryTime),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            super::machine_mcp_tests::authenticated_machine_chat(&f).await
        };
        let failed = result(&call(&f.state, &auth, json!({"text":"x".repeat(501)})).await);
        let original = failed["details"]["failed_activity_id"].as_str().unwrap();
        let mut args = json!({
            "text":"A business fact",
            "recovery":{"failed_activity_id":original,"part":1,"total":1}
        });
        let mut invalid = args.clone();
        invalid["recovery"]["part"] = json!(2);
        assert_eq!(
            result(&call(&f.state, &auth, invalid).await)["error"],
            "memory_recovery_unavailable"
        );
        let mut unrelated = args.clone();
        unrelated["recovery"]["failed_activity_id"] = json!(uuid::Uuid::new_v4());
        assert_eq!(
            result(&call(&f.state, &auth, unrelated).await)["error"],
            "memory_recovery_unavailable"
        );
        let mut forged = args.clone();
        forged["recovery"]["committed"] = json!(true);
        assert_eq!(
            result(&call(&f.state, &auth, forged).await)["details"]["reason"],
            "invalid_note"
        );
        assert!(
            acks::history(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap()
                .is_empty()
        );
        let card = result(&call(&f.state, &auth, args.clone()).await);
        assert_eq!(card["error"], "acknowledgement_required", "{card}");
        let id = card["acknowledgement_id"].as_str().unwrap();
        let mut second = args.clone();
        second["text"] = json!("The same business fact worded differently");
        let second_card = result(&call(&f.state, &auth, second.clone()).await);
        assert_eq!(
            second_card["error"], "acknowledgement_required",
            "{second_card}"
        );
        let second_id = second_card["acknowledgement_id"].as_str().unwrap();
        assert_ne!(id, second_id);
        let pending = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
        assert!(
            pending
                .active_turn
                .unwrap()
                .activities
                .iter()
                .all(|activity| { activity.memory_recovery.is_none() })
        );

        acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
            .await
            .unwrap();
        acks::decide(&f.state.db, &f.owner, &f.row.id, second_id, true)
            .await
            .unwrap();
        if continuation {
            // Model the live continuation snapshot after the original turn left
            // the active slot, retaining the trigger policy on the same thread.
            let next = uuid::Uuid::new_v4().to_string();
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.turn_id":&next,"active_turn.activities":[]}},
                )
                .await
                .unwrap();
            auth.chat.as_mut().unwrap().turn_id = Some(next);
        }
        args["acknowledgement_id"] = json!(id);
        let save = result(&call(&f.state, &auth, args.clone()).await);
        assert!(save["remembered"].is_string(), "{save}");
        if continuation {
            assert_eq!(save["recovery_status"], "unlinked_after_approval");
            assert!(save["recovery"].is_null());
        } else {
            assert_eq!(save["recovery"]["committed"], true);
        }
        let replay = call(&f.state, &auth, args).await;
        assert_eq!(replay["result"]["isError"], true, "{replay}");
        second["acknowledgement_id"] = json!(second_id);
        let duplicate = result(&call(&f.state, &auth, second).await);
        assert_eq!(duplicate["error"], "memory_recovery_unavailable");
        let agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
        assert_eq!(agent.memory.len(), 1);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn remember_recovery_refuses_an_uncertain_part_and_an_unrelated_turn() {
    let f = orchestrator_fixture("remember_recovery_uncertain").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    let failed = result(&call(&f.state, &auth, json!({"text": "x".repeat(501)})).await);
    let original = failed["details"]["failed_activity_id"].as_str().unwrap();
    let retry = json!({"text":"Business fact","recovery":{"failed_activity_id":original,"part":1,"total":1}});
    f.state
        .db
        .run_command(doc! {
            "collMod": AGENTS, "validator": {"memory": {"$size": 0}}, "validationLevel": "strict"
        })
        .await
        .unwrap();
    let uncertain = result(&call(&f.state, &auth, retry.clone()).await);
    assert_eq!(uncertain["details"]["reason"], "memory_storage_error");
    assert_eq!(uncertain["recovery"]["committed"], false);
    assert!(uncertain["details"]["failed_activity_id"].is_null());
    f.state
        .db
        .run_command(doc! {"collMod": AGENTS, "validator": {}})
        .await
        .unwrap();
    assert_eq!(
        result(&call(&f.state, &auth, retry.clone()).await)["error"],
        "memory_recovery_unavailable"
    );
    // A separate live turn for the same owner and agent cannot use this ID.
    let row = crate::services::assistant_authority_tests::new_orchestrator(
        &f.state,
        &f.owner,
        "A separate request",
    )
    .await;
    let mut other = auth.clone();
    other.chat = crate::services::assistant_acknowledgement_service::for_key(
        &f.state.db,
        &f.owner,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap();
    other.api_key_id = Some(row.credential_api_key_id.clone());
    assert_eq!(
        result(&call(&f.state, &other, retry).await)["error"],
        "memory_recovery_unavailable"
    );
    assert!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .memory
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remember_recovery_preserves_the_named_agent_and_original_replacement() {
    let f = orchestrator_fixture("remember_recovery_replace").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    let (agent, _) = team::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: "baker".into(),
            description: "Track cake orders".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "nyxbot",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let first = result(
        &call(
            &f.state,
            &auth,
            json!({"agent":"baker","text":"Old pickup time"}),
        )
        .await,
    );
    let second = result(
        &call(
            &f.state,
            &auth,
            json!({"agent":agent.id,"text":"Unrelated note"}),
        )
        .await,
    );
    let note = first["remembered"].as_str().unwrap();
    let other = second["remembered"].as_str().unwrap();
    let failed = result(
        &call(
            &f.state,
            &auth,
            json!({"agent":"baker","text":"x".repeat(501),"replace_id":note}),
        )
        .await,
    );
    let original = failed["details"]["failed_activity_id"].as_str().unwrap();
    let base = json!({"agent":agent.id,"text":"New pickup time","recovery":{"failed_activity_id":original,"part":1,"total":1}});
    let mut wrong = base.clone();
    wrong["replace_id"] = json!(other);
    for args in [base.clone(), wrong] {
        assert_eq!(
            result(&call(&f.state, &auth, args).await)["error"],
            "memory_recovery_unavailable"
        );
    }
    let mut correct = base;
    correct["replace_id"] = json!(note);
    let mut split = correct.clone();
    split["recovery"]["total"] = json!(2);
    assert_eq!(
        result(&call(&f.state, &auth, split).await)["error"],
        "memory_recovery_unavailable"
    );
    let saved = result(&call(&f.state, &auth, correct).await);
    assert_eq!(saved["remembered"], note);
    assert_eq!(saved["recovery"]["committed"], true);

    let notes = team::agent(&f.state.db, &f.owner, &agent.id)
        .await
        .unwrap()
        .memory;
    assert_eq!(notes.len(), 2);
    assert_eq!(
        notes.iter().find(|n| n.id == note).unwrap().text,
        "New pickup time"
    );
    assert_eq!(
        notes.iter().find(|n| n.id == other).unwrap().text,
        "Unrelated note"
    );
    let row = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": &f.row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !row.to_string().contains(note),
        "raw replacement ID belongs only in memory"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remember_recovery_with_unrecorded_commit_keeps_history_unconfirmed() {
    use crate::models::assistant_conversation::COLLECTION_NAME as CONVERSATIONS;
    use crate::services::assistant_nyxagent as engine;

    let f = orchestrator_fixture("remember_recovery_receipt_write").await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    let failed = result(&call(&f.state, &auth, json!({"text":"x".repeat(501)})).await);
    let original = failed["details"]["failed_activity_id"].as_str().unwrap();
    // Reject only the activity receipt after note storage succeeds.
    f.state.db.run_command(doc! {
        "collMod": CONVERSATIONS,
        "validator": {"active_turn.activities": {"$not": {"$elemMatch": {"memory_recovery.committed": true}}}},
    }).await.unwrap();
    let args = json!({"text":"Replace the rejected long note","recovery":{"failed_activity_id":original,"part":1,"total":1}});
    let receipt = result(&call(&f.state, &auth, args.clone()).await);
    assert!(receipt["remembered"].is_string());
    assert_eq!(receipt["recovery"]["committed"], true);
    assert_eq!(
        result(&call(&f.state, &auth, args).await)["error"],
        "memory_recovery_unavailable"
    );
    let message_id = uuid::Uuid::new_v4().to_string();
    engine::finish_turn(
        &f.state.db,
        &f.row,
        &f.row.credential_api_key_id,
        &message_id,
        &engine::TurnResult {
            text: "Done".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let (_, messages) = engine::history_page(&f.state.db, &f.owner, &f.row.id, 10, None)
        .await
        .unwrap();
    let message = messages
        .into_iter()
        .find(|message| message.id == message_id)
        .unwrap();
    let part = message
        .activities
        .iter()
        .find(|activity| activity.memory_recovery.is_some())
        .unwrap();
    assert_eq!(part.status, "completed");
    assert!(!part.memory_recovery.as_ref().unwrap().committed);
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
