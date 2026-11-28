//! Enrolled Discord follow authority through cold admission and mounted MCP.
use super::*;
use tower::ServiceExt;

#[tokio::test]
async fn enrolled_discord_org_follow_binds_hosted_admission_discovery_and_sender() {
    Box::pin(cold_guest_model_setup(true, true, true)).await;
}

pub(super) async fn live_discovery(
    router: Router,
    db: &mongodb::Database,
    row: &AssistantConversation,
    bearer: axum::http::HeaderValue,
) -> Value {
    let request = |method, params| {
        axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("authorization", bearer.clone())
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
            ))
            .unwrap()
    };
    let response = router
        .clone()
        .oneshot(request("tools/list", json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let discovery = serde_json::from_slice::<Value>(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    let search = router
        .clone()
        .oneshot(request(
            "tools/call",
            json!({"name":"nyx__search_tools","arguments":{"query":"nyx__resource_get"}}),
        ))
        .await
        .unwrap();
    assert_eq!(search.status(), StatusCode::OK);
    let search: Value = serde_json::from_slice(
        &axum::body::to_bytes(search.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(search["result"]["isError"], false, "{search}");
    let search: Value =
        serde_json::from_str(search["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        !search["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "nyx__resource_get")
    );
    let conversations = db.collection::<bson::Document>(CONVERSATIONS);
    let filter = doc! {"_id":&row.id,"credential_api_key_id":&row.credential_api_key_id};
    // The actual authenticated credential cannot move its hosted authority to
    // another partition. Restore the exact fixture row before model egress.
    assert_eq!(
        conversations
            .update_one(
                filter.clone(),
                doc! {"$set":{"channel.partition":"other-thread"}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
    let refusal = router
        .clone()
        .oneshot(request("tools/list", json!({})))
        .await
        .unwrap();
    assert!(matches!(
        refusal.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));
    assert_eq!(
        conversations
            .update_one(
                filter,
                doc! {"$set":{"channel.partition":&row.channel.as_ref().unwrap().partition}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
    let response = router
        .oneshot(request("tools/list", json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    discovery
}

pub(super) async fn assertions(
    state: &AppState,
    channel: &NyxbotChannel,
    org_id: &str,
    seen: &Value,
) {
    use crate::services::{channel_guest_access as guest, channel_thread_follow_service as follow};
    let discovery = &seen["discovery"];
    assert!(discovery.get("error").is_none(), "{discovery}");
    let names: Vec<_> = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&crate::services::assistant_tool_result_service::TOOL_NAME));
    for forbidden in [
        "nyx__resource_get",
        "nyx__conversation_history",
        "nyxid__list_services",
    ] {
        assert!(!names.contains(&forbidden), "{names:?}");
    }
    let row = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":seen["conversation_id"].as_str().unwrap(),"user_id":org_id})
        .await
        .unwrap()
        .unwrap();
    assert!(guest::is_hosted_conversation(&row.id));
    assert!(row.guest_turn);
    assert_eq!(
        row.credential_api_key_id,
        seen["api_key_id"].as_str().unwrap()
    );
    let turn: guest::GuestTurn =
        serde_json::from_value(seen["persisted_context"]["channel_guest"].clone()).unwrap();
    assert_eq!(
        turn.actor.assurance,
        guest::GuestAssurance::ProviderVerified
    );
    assert!(turn.allows_shared_sheets_write());
    assert!(turn.matches_sender("8"));
    assert!(!turn.matches_sender("9"));
    assert_eq!(turn.hosted_conversation_id(), row.id);
    let origin = row.channel.as_ref().unwrap();
    let child = follow::validate_delivery(&state.db, org_id, origin, &row.id)
        .await
        .unwrap();
    assert_eq!(child.channel_id, channel.id);
    assert_ne!(child.conversation_id.as_deref(), Some(row.id.as_str()));
    assert!(!guest::is_hosted_conversation_namespace(
        child.conversation_id.as_deref().unwrap()
    ));
    assert!(
        crate::services::org_channel_service::conversation_access(&state.db, &row)
            .await
            .unwrap()
            .is_some()
    );
    for mutation in ["sender", "partition", "source", "actor"] {
        let mut wrong = origin.clone();
        match mutation {
            "sender" => wrong.thread.as_mut().unwrap().sender_id = "9".into(),
            "partition" => wrong.partition = "different-thread".into(),
            "source" => {
                wrong.thread.as_mut().unwrap().source_message_id = "different-source".into()
            }
            _ => wrong.org_execution.as_mut().unwrap().actor_id = OWNER.into(),
        }
        assert!(
            follow::validate_delivery(&state.db, org_id, &wrong, &row.id)
                .await
                .is_err(),
            "{mutation}"
        );
    }
    let mut legacy = turn.clone();
    legacy.shared_room_verified = false;
    assert_ne!(legacy.hosted_conversation_id(), row.id);
    // Original-source sender continuity remains live after admission.
    let sources = state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME);
    let source = &origin.thread.as_ref().unwrap().source_message_id;
    assert_eq!(
        sources
            .update_one(
                doc! {"_id":source},
                doc! {"$set":{"sender_platform_id":"9"}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
    assert!(
        follow::validate_delivery(&state.db, org_id, origin, &row.id)
            .await
            .is_err()
    );
    assert_eq!(
        sources
            .update_one(
                doc! {"_id":source},
                doc! {"$set":{"sender_platform_id":"8"}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
    assert!(
        follow::validate_delivery(&state.db, org_id, origin, &row.id)
            .await
            .is_ok()
    );
}
