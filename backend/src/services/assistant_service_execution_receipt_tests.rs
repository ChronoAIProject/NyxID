use super::*;
use axum::{body::Body, response::IntoResponse, routing::any};
use futures::StreamExt;

pub(super) async fn require_approval(f: &Fixture, service: &str) {
    use crate::models::service_approval_config::{
        ApprovalMode, COLLECTION_NAME, ServiceApprovalConfig,
    };
    let now = Utc::now();
    f.state
        .db
        .collection::<ServiceApprovalConfig>(COLLECTION_NAME)
        .insert_one(ServiceApprovalConfig {
            id: Uuid::new_v4().to_string(),
            user_id: f.owner.clone(),
            service_id: service.into(),
            service_name: "Receipt target".into(),
            approval_required: true,
            approval_mode: ApprovalMode::PerRequest,
            rules: vec![],
            default_effect: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}
async fn receipts(f: &Fixture) -> Vec<Document> {
    f.state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .find(doc! {"conversation_id":&f.row.id,"decided_by":"access_mode"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

#[tokio::test]
async fn completion_receipt_selects_approval_and_rejects_failed_or_abandoned_streams() {
    let f = orchestrator_fixture("receipt_selection_eof").await;
    mode(&f, AccessMode::Autopilot).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "receipt", None).await;
    for outcome in ["clean", "failed", "dropped"] {
        let decision = gate(&f, &id, &owner, &json!({"path":outcome}), None).await;
        assert!(
            decision.automatic.is_some(),
            "exercise overlapping automatic decisions"
        );
        let source = futures::stream::iter(vec![Ok::<_, std::io::Error>(
            bytes::Bytes::from_static(b"part"),
        )]);
        let body = match outcome {
            "failed" => Body::from_stream(source.chain(futures::stream::once(async {
                Err(std::io::Error::other("upstream interrupted"))
            }))),
            "dropped" => Body::from_stream(source.chain(futures::stream::pending())),
            _ => Body::from_stream(source),
        };
        let response = axum::response::Response::builder()
            .header("content-length", "4")
            .body(body)
            .unwrap();
        let approval = HttpApprovalReceipt::new(
            Some(acks::AutoAllowed {
                class: acks::OwnerActionClass::Read,
                mode: AccessMode::Autopilot,
                reason: "approval",
            }),
            &id,
            "Receipt target",
            &crate::services::operation_descriptor::build_llm_descriptor("GET", "/read", None),
        );
        let response = decision
            .finish_http(
                response,
                f.state.db.clone(),
                Some(f.chat.clone()),
                "GET",
                approval,
            )
            .await;
        assert!(!response.headers().contains_key("content-length"));
        if outcome == "dropped" {
            drop(response);
        } else {
            let result = axum::body::to_bytes(response.into_body(), 1000).await;
            assert_eq!(result.is_ok(), outcome == "clean");
        }
        let rows = receipts(&f).await;
        assert_eq!(rows.len(), 1, "{outcome}");
        assert_eq!(rows[0].get_str("tool_name"), Ok("service_approval"));
        assert_eq!(
            rows[0]
                .get_document("confirmation")
                .unwrap()
                .get_str("reason"),
            Ok("approval")
        );
    }
}

#[tokio::test]
async fn actual_http_receipts_choose_one_approval_on_success_head_and_bodyless_statuses() {
    let f = orchestrator_fixture("http_receipt_lifecycle").await;
    mode(&f, AccessMode::Autopilot).await;
    let owner = org(&f).await;
    let upstream = axum::Router::new().fallback(any(|uri: axum::http::Uri| async move {
        match uri.path() {
            "/empty" => axum::http::StatusCode::NO_CONTENT.into_response(),
            "/reset" => axum::http::StatusCode::RESET_CONTENT.into_response(),
            "/fail" => axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
            "/stream" => {
                let source = futures::stream::once(async {
                    Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"data: ready\n\n"))
                })
                .chain(futures::stream::pending());
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(source))
                    .unwrap()
            }
            _ => "complete".into_response(),
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let id = connected(&f.state.db, &owner, "receipt-target", &base).await;
    require_approval(&f, &id).await;
    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (_, router) = crate::routes::build_router_with_state(f.state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/v1/proxy/s/receipt-target",
        listener.local_addr().unwrap()
    );
    let router = router.with_state(f.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    for (index, (method, path, status)) in [
        ("GET", "ok", 200),
        ("HEAD", "ok", 200),
        ("POST", "empty", 204),
        ("POST", "reset", 205),
    ]
    .into_iter()
    .enumerate()
    {
        let response = client
            .request(method.parse().unwrap(), format!("{base}/{path}"))
            .bearer_auth(credential.raw_key.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status, "{method} {path}");
        if method == "GET" {
            assert!(!response.headers().contains_key("content-length"));
        }
        response.bytes().await.unwrap();
        let rows = receipts(&f).await;
        assert_eq!(rows.len(), index + 1, "{method} {path}");
        assert!(
            rows.iter()
                .all(|row| row.get_str("tool_name") == Ok("service_approval")
                    && row.get_document("confirmation").unwrap().get_str("reason")
                        == Ok("approval"))
        );
    }
    let failed = client
        .get(format!("{base}/fail"))
        .bearer_auth(credential.raw_key.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status().as_u16(), 503);
    failed.bytes().await.unwrap();
    let mut abandoned = client
        .get(format!("{base}/stream"))
        .bearer_auth(credential.raw_key.as_str())
        .send()
        .await
        .unwrap()
        .bytes_stream();
    assert!(abandoned.next().await.unwrap().is_ok());
    drop(abandoned);
    assert_eq!(receipts(&f).await.len(), 4);
    server.abort();
    provider.abort();
}

#[tokio::test]
async fn streamed_receipt_keeps_the_tool_call_it_ran_in() {
    let f = orchestrator_fixture("receipt_activity_id").await;
    mode(&f, AccessMode::Autopilot).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "receipt", None).await;
    let decision = gate(&f, &id, &owner, &json!({"path":"clean"}), None).await;
    assert!(decision.automatic.is_some());
    let source = futures::stream::iter(vec![Ok::<_, std::io::Error>(bytes::Bytes::from_static(
        b"part",
    ))]);
    let response = axum::response::Response::builder()
        .body(Body::from_stream(source))
        .unwrap();
    // The receipt is chosen while the tool call runs; its body reaches EOF
    // after that scope has ended, as Hyper polls it.
    let response = crate::services::machine_activity_service::ACTIVITY_ID
        .scope(
            Some("activity-1".into()),
            decision.finish_http(
                response,
                f.state.db.clone(),
                Some(f.chat.clone()),
                "GET",
                None,
            ),
        )
        .await;
    axum::body::to_bytes(response.into_body(), 1000)
        .await
        .unwrap();
    let rows = receipts(&f).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get_str("activity_id").ok(), Some("activity-1"));
}
