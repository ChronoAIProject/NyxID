//! Enrolled Discord follow authority through cold admission and mounted MCP.
use super::*;
use tower::ServiceExt;

#[tokio::test]
async fn enrolled_discord_org_follow_binds_hosted_admission_discovery_and_sender() {
    Box::pin(cold_guest_model_setup(true, true, true)).await;
}

pub(super) async fn pending_authority(state: &AppState, setup: &AssistantConversation) {
    use crate::{
        models::nyxbot_channel::WaitingGuest, services::channel_thread_follow_service as follow,
    };
    let event_id = setup
        .channel
        .as_ref()
        .unwrap()
        .native_event_id
        .as_deref()
        .unwrap();
    let source = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let event = state
                .db
                .collection::<bson::Document>(EVENTS)
                .find_one(doc! {"_id":event_id})
                .await
                .unwrap()
                .unwrap();
            if event.contains_key(event_record::CHANNEL_GUEST) {
                break event;
            }
            assert_ne!(
                event.get_str("status"),
                Ok("refused"),
                "source retired before setup dispatch"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the source waits for this model setup");
    assert_eq!(source.get_str("status"), Ok("queued"));
    assert!(
        source
            .get("turn_id")
            .is_none_or(|id| id == &bson::Bson::Null)
    );
    let bytes = event_record::guest_setup(&state.encryption_keys, &source)
        .await
        .unwrap();
    let payload: WaitingGuest = serde_json::from_slice(&bytes).unwrap();
    let guest = payload
        .channel_context
        .as_ref()
        .unwrap()
        .channel_guest
        .as_ref()
        .unwrap();
    assert_eq!(guest.source_event_id, event_id);
    assert_eq!(guest.hosted_conversation_id(), payload.conversation);
    assert!(guest.matches_sender("8"));
    let actor = &payload.origin.org_execution.as_ref().unwrap().actor_id;
    let child = follow::validate_pending_hosted_delivery(&state.db, actor, &payload.origin, guest)
        .await
        .unwrap();
    assert_eq!(child.id, payload.origin.thread.as_ref().unwrap().child_id);
    for mutation in ["sender", "source", "actor", "partition"] {
        let mut wrong = payload.origin.clone();
        match mutation {
            "sender" => wrong.thread.as_mut().unwrap().sender_id = "9".into(),
            "source" => wrong.thread.as_mut().unwrap().source_message_id = "other-source".into(),
            "actor" => wrong.org_execution.as_mut().unwrap().actor_id = OWNER.into(),
            _ => wrong.partition = "other-thread".into(),
        }
        assert!(
            follow::validate_pending_hosted_delivery(&state.db, actor, &wrong, guest)
                .await
                .is_err(),
            "{mutation}"
        );
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("channel_guest_work_v1")
            .count_documents(doc! {"turn.source_event_id":event_id})
            .await
            .unwrap(),
        0
    );
}

pub(super) async fn diagnostic(state: &AppState, channel: &NyxbotChannel) -> Value {
    let event=state.db.collection::<bson::Document>(EVENTS).find_one(doc!{"channel_id":&channel.id,"event_id":"discord:100:502"})
        .projection(doc!{"status":1,"turn_id":1,"conversation_id":1,"channel_guest_admission":1,"delivery.state":1}).await.unwrap();
    let setup=state.db.collection::<bson::Document>(crate::services::assistant_service_execution::COLLECTION)
        .find_one(doc!{"kind":crate::services::assistant_guest_model_setup::KIND,"agent_id":&channel.agent_id})
        .projection(doc!{"status":1,"reason":1,"failure_step":1}).await.unwrap();
    json!({"event":event,"setup":setup})
}

pub(super) async fn live_discovery(
    router: Router,
    db: &mongodb::Database,
    row: &AssistantConversation,
    bearer: axum::http::HeaderValue,
) -> Value {
    // NyxAgent's NyxID toolbox forwards the model bearer key as x-api-key
    // for MCP (docs/chat/08-nyxagent-engine.md, Upstream contract trace).
    let api_key = bearer
        .to_str()
        .unwrap()
        .strip_prefix("Bearer ")
        .expect("the model callback uses bearer authentication");
    let request = |method, params| {
        axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("x-api-key", api_key)
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
    let discovery =
        discovery_response(response, api_key, "initial tools/list", StatusCode::OK).await;
    // The model proxy uses bearer authentication separately. Its header cannot
    // acquire MCP authority through raw-key bearer/session fallback.
    let mut bearer_request = request("tools/list", json!({}));
    bearer_request.headers_mut().remove("x-api-key");
    bearer_request
        .headers_mut()
        .insert("authorization", bearer.clone());
    let refusal = router.clone().oneshot(bearer_request).await.unwrap();
    let refusal = discovery_response(
        refusal,
        api_key,
        "raw-key bearer tools/list",
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_eq!(refusal["error"]["code"], -32003);
    assert_eq!(
        refusal["error"]["message"],
        "API key is missing the required scope for MCP access. Expected: proxy"
    );
    assert!(refusal.get("result").is_none());
    let search = router
        .clone()
        .oneshot(request(
            "tools/call",
            json!({"name":"nyx__search_tools","arguments":{"query":"nyx__resource_get"}}),
        ))
        .await
        .unwrap();
    let search = discovery_response(search, api_key, "native tool search", StatusCode::OK).await;
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
    discovery_response(response, api_key, "restored tools/list", StatusCode::OK).await;
    discovery
}

async fn discovery_response(
    response: axum::response::Response,
    api_key: &str,
    stage: &str,
    expected: StatusCode,
) -> Value {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("mounted MCP response fits the fixture limit");
    let body: Value = serde_json::from_slice(&bytes).expect("mounted MCP returns JSON");
    // Retain actionable refusal details without logging credentials or the
    // complete tool/result payload. Cover both JSON-RPC and HTTP error shapes.
    let diagnostic = json!({
        "code":body.pointer("/error/code").or_else(|| body.get("code")),
        "error":body.get("error").and_then(Value::as_str),
        "message":body.pointer("/error/message").or_else(|| body.get("message")),
    })
    .to_string()
    .replace(api_key, "[redacted]");
    assert_eq!(status, expected, "{stage}: {diagnostic}");
    if expected.is_success() {
        assert!(body.get("error").is_none(), "{stage}: {diagnostic}");
    }
    body
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
