//! Mounted MCP/proxy checks using the same real Google catalogs as discovery.
use super::{AnswerAudience, test_support::*};
use crate::services::{
    assistant_agent_credential_service as credentials, assistant_authority_tests::Fixture,
    assistant_tool_result_service as retained, destination_routing::tests as google,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use bson::{Document, doc};
use serde_json::{Value, json};
use tower::ServiceExt;

fn calls(name: &str, arguments: &Value) -> [Value; 2] {
    [
        json!({"name":name,"arguments":arguments}),
        json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":arguments.to_string()}}),
    ]
}

fn text(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

async fn raw_append(router: &axum::Router, key: &str, slug: &str) -> StatusCode {
    router.clone().oneshot(Request::builder().method("POST")
        .uri(format!("/api/v1/proxy/s/{slug}/v4/spreadsheets/private_workbook_2704/values/Hidden_Questions!A:D:append?valueInputOption=RAW&insertDataOption=INSERT_ROWS"))
        .header("x-api-key",key).header("content-type","application/json")
        .body(Body::from(json!({"values":[["public question"]]}).to_string())).unwrap())
        .await.unwrap().status()
}

pub(super) async fn exercise(f: &Fixture, conversation: &str) {
    let db = &f.state.db;
    let key =
        credentials::load_for_conversation(db, &f.state.encryption_keys, &f.owner, conversation)
            .await
            .unwrap()
            .unwrap();
    let values = json!({"range":"Hidden_Config!B2:D5","values":[[format!("{}answer after the first page", "event detail ".repeat(2000))]]});
    let expected_text = values.to_string();
    let echo = google::Echo::start_with_sheets_values(values.clone()).await;
    let mut state = f.state.clone();
    state.http_client = echo.client.clone();
    let (_, router) = crate::routes::build_router_with_state(state.clone());
    let router = router.with_state(state);
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
        .scope(
            echo.client_builder.clone(),
            Box::pin(exercise_routes(
                f,
                &router,
                conversation,
                key.raw_key.as_str(),
                &echo,
                &expected_text,
            )),
        )
        .await;
}

async fn exercise_routes(
    f: &Fixture,
    router: &axum::Router,
    conversation: &str,
    key: &str,
    echo: &google::Echo,
    expected_text: &str,
) {
    let db = &f.state.db;

    let list = mcp_request(router, key, "tools/list", json!({})).await;
    let names: Vec<_> = list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&retained::TOOL_NAME));
    assert!(!names.contains(&"nyx__resource_get"));
    for query in [retained::TOOL_NAME, "nyx__resource_get"] {
        let search = text(
            &mcp_request(
                router,
                key,
                "tools/call",
                json!({"name":"nyx__search_tools","arguments":{"query":query}}),
            )
            .await,
        );
        assert!(
            !search["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "nyx__resource_get"),
            "{search}"
        );
        if query == retained::TOOL_NAME {
            assert_eq!(search["matches"][0]["name"], retained::TOOL_NAME);
        }
    }
    for call in calls("nyx__resource_get", &json!({"id":"unavailable"})) {
        assert_eq!(
            mcp_request(router, key, "tools/call", call).await["isError"],
            true
        );
    }

    let args = json!({"spreadsheetId":"private_workbook_2704","range":"Hidden_Questions!A:D",
            "valueInputOption":"RAW","insertDataOption":"INSERT_ROWS","body":{"values":[["public question"]]}});
    // Discovery does not bypass the real default write approval decision.
    assert_eq!(
        raw_append(router, key, "api-google-sheets").await,
        StatusCode::FORBIDDEN
    );
    assert!(echo.calls.lock().unwrap().is_empty());
    assert_eq!(
        db.collection::<Document>(crate::models::notification_channel::COLLECTION_NAME)
            .update_one(
                doc! {"user_id":&f.owner},
                doc! {"$set":{"approval_required":false}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
    let mut last_page = Value::Null;
    for slug in ["api-google-workspace", "api-google-sheets"] {
        let append = format!("{slug}__sheets_append_values");
        let before_invalid = echo.calls.lock().unwrap().len();
        for (field, value) in [
            ("responseValueRenderOption", json!("FORMATTED_VALUE")),
            ("responseDateTimeRenderOption", json!("SERIAL_NUMBER")),
            ("unsupported", json!(true)),
        ] {
            let mut bad = args.clone();
            bad[field] = value;
            for call in calls(&append, &bad) {
                assert_eq!(
                    mcp_request(router, key, "tools/call", call).await["isError"],
                    true
                );
            }
        }
        let mut bad_body = args.clone();
        bad_body["body"]["range"] = json!("Hidden_Questions!A:D");
        for call in calls(&append, &bad_body) {
            assert_eq!(
                mcp_request(router, key, "tools/call", call).await["isError"],
                true
            );
        }
        assert_eq!(echo.calls.lock().unwrap().len(), before_invalid);
        let before = echo.calls.lock().unwrap().len();
        for call in calls(&append, &args) {
            let result = mcp_request(router, key, "tools/call", call).await;
            assert_eq!(result["isError"], false, "{result}");
        }
        assert_eq!(raw_append(router, key, slug).await, StatusCode::OK);
        assert_eq!(echo.calls.lock().unwrap().len(), before + 3);
        let read = format!("{slug}__sheets_get_values");
        for call in calls(
            &read,
            &json!({"spreadsheetId":"private_workbook_2704","range":"Hidden_Config!B2:D5"}),
        ) {
            let result = mcp_request(router, key, "tools/call", call).await;
            assert_eq!(result["isError"], false, "{result}");
            assert!(result.to_string().len() <= retained::BUDGET);
            let first = text(&result);
            assert!(first["parts"].as_u64().unwrap() > 1, "{first}");
            assert!(
                first["instructions"]
                    .as_str()
                    .unwrap()
                    .contains(retained::TOOL_NAME)
            );
            assert!(
                !first["content"]
                    .as_str()
                    .unwrap()
                    .contains("answer after the first page")
            );
            let mut full = first["content"].as_str().unwrap().to_owned();
            let sent = echo.calls.lock().unwrap().len();
            for part in 1..first["parts"].as_u64().unwrap() {
                let request = calls(
                    retained::TOOL_NAME,
                    &json!({"ref":first["ref"],"part":part}),
                );
                let result = mcp_request(
                    router,
                    key,
                    "tools/call",
                    request[(part % 2) as usize].clone(),
                )
                .await;
                assert_eq!(result["isError"], false, "{result}");
                assert!(result.to_string().len() <= retained::BUDGET);
                let page = text(&result);
                full.push_str(page["content"].as_str().unwrap());
            }
            // The service result's original text is JSON, reproduced byte for byte.
            assert_eq!(full, expected_text);
            assert_eq!(full.len(), first["total_bytes"].as_u64().unwrap() as usize);
            assert_eq!(echo.calls.lock().unwrap().len(), sent);
            last_page = first;
        }
    }
    // Other actors/destinations cannot borrow a retained result. Both private
    // and ambiguous shared turns keep bounded reads but cannot publish rows.
    for (sender, audience, proof) in [
        ("private-runtime", AnswerAudience::Private, false),
        ("unknown-runtime", AnswerAudience::Shared, false),
    ] {
        let chat = Box::pin(start_guest_with_evidence(
            f, sender, sender, audience, proof,
        ))
        .await;
        if sender == "unknown-runtime" {
            legacy_shared_evidence(db, &chat).await;
        }
        let other = credentials::load_for_conversation(
            db,
            &f.state.encryption_keys,
            &f.owner,
            &chat.conversation_id,
        )
        .await
        .unwrap()
        .unwrap();
        let read=mcp_request(router,other.raw_key.as_str(),"tools/call",json!({"name":"api-google-sheets__sheets_get_values","arguments":{"spreadsheetId":"private_workbook_2704","range":"Hidden_Config!B2:D5"}})).await;
        assert_eq!(read["isError"], false, "{read}");
        assert!(text(&read)["parts"].as_u64().unwrap() > 1);
        let before = echo.calls.lock().unwrap().len();
        for slug in ["api-google-workspace", "api-google-sheets"] {
            assert_eq!(
                raw_append(router, other.raw_key.as_str(), slug).await,
                StatusCode::FORBIDDEN
            );
            for call in calls(&format!("{slug}__sheets_append_values"), &args) {
                assert_eq!(
                    mcp_request(router, other.raw_key.as_str(), "tools/call", call).await["isError"],
                    true
                );
            }
        }
        for call in calls(
            retained::TOOL_NAME,
            &json!({"ref":last_page["ref"],"part":1}),
        ) {
            assert_eq!(
                mcp_request(router, other.raw_key.as_str(), "tools/call", call).await["isError"],
                true
            );
        }
        assert_eq!(echo.calls.lock().unwrap().len(), before);
    }
    // A later source rotates the credential even for the same guest/room.
    let row = crate::services::assistant_nyxagent::get(db, &f.owner, conversation)
        .await
        .unwrap();
    Box::pin(f.finish_turn(&row)).await;
    let next = Box::pin(start_guest_with_evidence(
        f,
        "shared-all",
        "shared-all-next",
        AnswerAudience::Shared,
        true,
    ))
    .await;
    assert_eq!(next.conversation_id, conversation);
    let renewed =
        credentials::load_for_conversation(db, &f.state.encryption_keys, &f.owner, conversation)
            .await
            .unwrap()
            .unwrap();
    assert_ne!(next.api_key_id, row.credential_api_key_id);
    for call in calls(
        retained::TOOL_NAME,
        &json!({"ref":last_page["ref"],"part":1}),
    ) {
        assert_eq!(
            mcp_request(router, renewed.raw_key.as_str(), "tools/call", call).await["isError"],
            true
        );
    }
    let (status, _) = mcp_response(router, key, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let fresh=mcp_request(router,renewed.raw_key.as_str(),"tools/call",json!({"name":"api-google-sheets__sheets_get_values","arguments":{"spreadsheetId":"private_workbook_2704","range":"Hidden_Config!B2:D5"}})).await;
    assert_eq!(fresh["isError"], false, "{fresh}");
    last_page = text(&fresh);
    let before_revoke = echo.calls.lock().unwrap().len();
    // A policy revision invalidates cached data along with live operations.
    assert_eq!(
        db.collection::<Document>(crate::models::channel_guest_policy::COLLECTION_NAME)
            .update_one(doc! {"_id":"bot-1"}, doc! {"$inc":{"revision":1_i64}})
            .await
            .unwrap()
            .matched_count,
        1
    );
    for call in calls(
        retained::TOOL_NAME,
        &json!({"ref":last_page["ref"],"part":1}),
    ) {
        let (status, body) =
            mcp_response(router, renewed.raw_key.as_str(), "tools/call", call).await;
        assert!(
            matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
                || (status == StatusCode::OK
                    && (body["error"]["code"] == -32001
                        || (body["result"]["isError"] == true
                            && body["result"]["content"][0]["text"].as_str().is_some_and(
                                |text| text.contains("unavailable or expired for this turn")
                            )))),
            "{status}: {body}"
        );
    }

    assert_eq!(echo.calls.lock().unwrap().len(), before_revoke);
}
