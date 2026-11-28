//! Shared hosted guest admission and authenticated MCP test helpers.
use super::AnswerAudience;
use bson::{Document, doc};
use serde_json::{Value, json};

pub(super) async fn start_guest(
    f: &crate::services::assistant_authority_tests::Fixture,
    sender: &str,
    event: &str,
) -> crate::services::assistant_acknowledgement_service::ChatAuthority {
    Box::pin(start_guest_with_audience(
        f,
        sender,
        event,
        AnswerAudience::Private,
    ))
    .await
}

pub(super) async fn start_guest_with_audience(
    f: &crate::services::assistant_authority_tests::Fixture,
    sender: &str,
    event: &str,
    audience: AnswerAudience,
) -> crate::services::assistant_acknowledgement_service::ChatAuthority {
    Box::pin(start_guest_with_evidence(
        f,
        sender,
        event,
        audience,
        audience == AnswerAudience::Shared,
    ))
    .await
}

pub(super) async fn start_guest_with_evidence(
    f: &crate::services::assistant_authority_tests::Fixture,
    sender: &str,
    event: &str,
    audience: AnswerAudience,
    shared_room_verified: bool,
) -> crate::services::assistant_acknowledgement_service::ChatAuthority {
    Box::pin(start_guest_with_input(
        f,
        super::VerifiedGuestInput {
            bot_id: "bot-1",
            binding_id: "binding",
            platform: "discord",
            conversation_id: sender,
            source_event_id: event,
            sender_id: Some(sender),
            audience,
            shared_room_verified,
            meta_grant_id: None,
        },
    ))
    .await
}

pub(super) async fn start_guest_with_input(
    f: &crate::services::assistant_authority_tests::Fixture,
    input: super::VerifiedGuestInput<'_>,
) -> crate::services::assistant_acknowledgement_service::ChatAuthority {
    use crate::models::{
        assistant_conversation::{
            COLLECTION_NAME as CONVERSATIONS, ChannelOrigin, ChannelTurnContext, TurnOrigin,
        },
        channel_thread::ThreadSenderKind,
    };
    use crate::services::{
        assistant_authority_tests::chat_for_row,
        assistant_nyxagent::{self as engine, TurnRequest, TurnStart},
        channel_guest_access::{self as guest, GuestAdmission},
    };
    let db = &f.state.db;
    let event = input.source_event_id;
    let audience = input.audience;
    let platform = input.platform;
    let partition = input.conversation_id;
    let bot_id = input.bot_id;
    let binding_id = input.binding_id;
    let mode = guest::select_mode(db, bot_id).await.unwrap();
    let GuestAdmission::AddOn(turn) = guest::classify(db, mode, input).await.unwrap() else {
        panic!("expected the guest add-on admission");
    };
    let id = turn.hosted_conversation_id();
    let existing = db
        .collection::<Document>(CONVERSATIONS)
        .find_one(doc! {"_id":&id})
        .projection(doc! {"_id":1})
        .await
        .unwrap()
        .is_some();
    let mut start = TurnStart::from(&TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: existing.then(|| id.clone()),
        agent_id: f.row.agent_id.clone(),
        text: "Show my personal records".into(),
        model: None,
        access_mode: None,
    });
    start.guest = true;
    start.origin = TurnOrigin::Channel;
    start.channel_event_id = Some(event.into());
    start.new_id = Some(id);
    start.channel = Some(ChannelOrigin {
        org_execution: None,
        native_event_id: None,
        thread: None,
        nyxbot_channel_id: binding_id.into(),
        partition: partition.into(),
        platform: platform.into(),
    });
    start.channel_context = Some(ChannelTurnContext {
        channel_guest: Some(*turn),
        meta_grant: None,
        text_completeness: None,
        org_origin: None,
        display_name: None,
        conversation_label: None,
        followed: false,
        source_event_id: event.into(),
        channel_id: binding_id.into(),
        channel_bot_id: bot_id.into(),
        conversation_kind: if audience == AnswerAudience::Private {
            "private"
        } else {
            "group"
        }
        .into(),
        sender_kind: ThreadSenderKind::Human,
        guest: true,
        original_message: true,
    });
    db.collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .insert_one(doc! {
            "_id":event, "user_id":&f.owner, "status":"running",
            "delivery":{"version":1},
        })
        .await
        .unwrap();
    let row = Box::pin(engine::begin_turn(
        db,
        &f.owner,
        start,
        &f.state.encryption_keys,
    ))
    .await
    .unwrap();
    chat_for_row(&f.state, &f.owner, &row).await
}

/// Model an existing server-authored Shared turn from before room proof existed.
pub(super) async fn legacy_shared_evidence(
    db: &mongodb::Database,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
) {
    let turn = chat.channel_guest.as_ref().unwrap();
    assert!(!turn.shared_room_verified);
    assert_eq!(
        db.collection::<Document>(super::WORK)
            .update_one(
                doc! {"turn.bot_id":&turn.bot_id,"turn.source_event_id":&turn.source_event_id},
                doc! {"$unset":{"turn.shared_room_verified":""}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
    assert_eq!(db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(
        doc!{"_id":&chat.conversation_id,"active_turn.turn_id":&chat.turn_id},
        doc!{"$unset":{"active_turn.channel_guest.shared_room_verified":"","active_turn.channel_context.channel_guest.shared_room_verified":""}}).await.unwrap().matched_count,1);
}

pub(super) async fn mcp_response(
    router: &axum::Router,
    key: &str,
    method: &str,
    params: Value,
) -> (axum::http::StatusCode, Value) {
    mcp_response_with_limit(router, key, method, params, 256 * 1024).await
}

async fn mcp_response_with_limit(
    router: &axum::Router,
    key: &str,
    method: &str,
    params: Value,
    response_limit: usize,
) -> (axum::http::StatusCode, Value) {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("x-api-key", key)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    if !status.is_success() {
        return (status, Value::Null);
    }
    let bytes = axum::body::to_bytes(response.into_body(), response_limit)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// A full seeded owner catalog includes native tools and Google operations.
/// Keep its test reader bounded without narrowing the discovery control.
pub(super) async fn seeded_tools_list(router: &axum::Router, key: &str) -> Value {
    let (status, body) =
        mcp_response_with_limit(router, key, "tools/list", json!({}), 2 * 1024 * 1024).await;
    successful_mcp_result(status, body)
}

fn successful_mcp_result(status: axum::http::StatusCode, body: Value) -> Value {
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert!(body.get("error").is_none(), "{body}");
    body["result"].clone()
}

pub(super) async fn mcp_request(
    router: &axum::Router,
    key: &str,
    method: &str,
    params: Value,
) -> Value {
    let (status, body) = mcp_response(router, key, method, params).await;
    successful_mcp_result(status, body)
}
