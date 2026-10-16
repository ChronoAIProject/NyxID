//! Supported model compaction retains exact account and action authority.
use super::compatibility::{flag, grant, idle};
use super::*;
use crate::services::{
    assistant_agent_credential_service as credentials, assistant_nyxagent as engine,
    platform_manager_contract as contract,
};
use axum::{body::Body, http::Request, response::Response};
use tower::ServiceExt;

async fn compact_connection(
    f: &Fixture,
    owner: &str,
    catalog: &str,
    slug: &str,
    url: &str,
) -> String {
    let id = connected(&f.state.db, owner, slug, url).await;
    let key = Uuid::new_v4().to_string();
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(Uuid::new_v4().to_string().as_bytes())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_api_keys")
        .insert_one(doc! {
            "_id":&key,"user_id":owner,"label":slug,"credential_type":"api_key",
            "status":"active","credential_encrypted":bson::Binary{
                subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"catalog_service_id":catalog,"api_key_id":key,
            "auth_method":"bearer","auth_key_name":"Authorization"}},
        )
        .await
        .unwrap();
    id
}

async fn compact_call(
    f: &Fixture,
    slug: &str,
    body: &Value,
    acknowledgement: Option<&str>,
) -> (u16, Value) {
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/proxy/s/{slug}/v1/responses/compact"))
        .header("content-type", "application/json")
        .header("x-api-key", key.raw_key.as_str());
    if let Some(id) = acknowledgement {
        request = request.header(ACK_HEADER, id);
    }
    let (_, router) = crate::routes::build_router();
    let response = router
        .with_state(f.state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn personal_compaction_preserves_live_and_idle_model_scope_without_account_fallback() {
    for live in [true, false] {
        let mut f = orchestrator_fixture("compact_model_scope").await;
        flag(&f, false).await;
        mode(&f, AccessMode::Nyxmode).await;
        let catalog = catalog(&f).await;
        let a = wiremock::MockServer::start().await;
        let b = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/responses/compact"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(
                    json!({"id":"cmp_test","object":"response.compaction","output":[]}),
                ),
            )
            .mount(&a)
            .await;
        let first = compact_connection(&f, &f.owner, &catalog, "compact-a", &a.uri()).await;
        compact_connection(&f, &f.owner, &catalog, "compact-b", &b.uri()).await;
        // Model transport remains reachable without adding it to tool grants.
        grant(&f, &Uuid::new_v4().to_string()).await;
        f.state
            .db
            .collection::<Document>("service_preferences")
            .insert_one(doc! {
                "_id":&f.owner,"ordered":[&first],"created_at":bson::DateTime::now(),
                "updated_at":bson::DateTime::now(),
            })
            .await
            .unwrap();
        if live {
            choose(&f, &first).await;
        } else {
            idle(&mut f).await;
        }
        let body = json!({"model":"m","input":[{"type":"input_file","file_id":"file_account_a"}]});
        assert!(!stateless_inference("v1/responses/compact", &body));
        let (status, result) = compact_call(&f, "compact-a", &body, None).await;
        assert_eq!(status, 200, "live={live}: {result}");
        let (status, result) = compact_call(&f, "compact-b", &body, None).await;
        assert_eq!(status, 409, "live={live}: {result}");
        assert_eq!(
            result["error"],
            if live {
                "service_selection_required"
            } else {
                "service_target_selection_required"
            },
            "{result}",
        );
        assert_eq!(a.received_requests().await.unwrap().len(), 1);
        assert!(b.received_requests().await.unwrap().is_empty());
        assert_eq!(
            f.state
                .db
                .collection::<Document>(COLLECTION)
                .count_documents(doc! {"kind":{"$in":["model_consent","unavailable_bucket"]}})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn compaction_approval_keeps_exact_account_and_semantic_body() {
    let f = orchestrator_fixture("compact_exact_consent").await;
    flag(&f, false).await;
    mode(&f, AccessMode::AskEveryTime).await;
    let catalog = catalog(&f).await;
    let a = wiremock::MockServer::start().await;
    let b = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"output":[]})))
        .mount(&a)
        .await;
    let first = compact_connection(&f, &f.owner, &catalog, "compact-a", &a.uri()).await;
    compact_connection(&f, &f.owner, &catalog, "compact-b", &b.uri()).await;
    grant(&f, &Uuid::new_v4().to_string()).await;
    choose(&f, &first).await;
    let body = json!({"model":"m","input":"exact account context"});
    let (status, refusal) = compact_call(&f, "compact-a", &body, None).await;
    assert_eq!(status, 409, "{refusal}");
    assert_eq!(refusal["error"], "acknowledgement_required", "{refusal}");
    let acknowledgement = allow(&f, refusal).await;
    let (status, refusal) = compact_call(
        &f,
        "compact-a",
        &json!({"model":"m","input":"different context"}),
        Some(&acknowledgement),
    )
    .await;
    assert_eq!(status, 409, "{refusal}");
    assert_eq!(refusal["error"], "acknowledgement_invalid", "{refusal}");
    let (status, refusal) = compact_call(
        &f,
        "compact-a",
        &json!({"model":"another-model","input":"exact account context"}),
        Some(&acknowledgement),
    )
    .await;
    assert_eq!(status, 409, "{refusal}");
    assert_eq!(refusal["error"], "acknowledgement_invalid", "{refusal}");
    let (status, refusal) = compact_call(&f, "compact-b", &body, Some(&acknowledgement)).await;
    assert_eq!(status, 409, "{refusal}");
    assert_eq!(refusal["error"], "service_selection_required", "{refusal}");
    assert!(a.received_requests().await.unwrap().is_empty());
    assert!(b.received_requests().await.unwrap().is_empty());
    let (status, result) = compact_call(&f, "compact-a", &body, Some(&acknowledgement)).await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(a.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .count_documents(doc! {"kind":"model_consent"})
            .await
            .unwrap(),
        0
    );
}

struct CompactEgress<'a> {
    fixture: &'a Fixture,
    body: Value,
}

#[async_trait::async_trait]
impl contract::NyxAgentEgress for CompactEgress<'_> {
    async fn request(
        &self,
        _: &contract::NyxAgentExecutionContext,
        _: &contract::RuntimeCredential,
        request: contract::NyxAgentRequest,
    ) -> AppResult<Response> {
        if request.method != "POST" {
            return Ok(Response::new(Body::from("{}")));
        }
        let (status, result) = compact_call(self.fixture, "compact-org", &self.body, None).await;
        if status == 409 {
            assert_eq!(result["error"], "acknowledgement_required", "{result}");
            return Err(AppError::Internal(
                "model compaction callback refused".into(),
            ));
        }
        assert_eq!(status, 200, "{result}");
        let terminal = json!({"type":"response.completed","sequence_number":1,"response":{
            "id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef",
            "conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},"status":"completed",
            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Compacted reply"}]}]}});
        Ok(Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!("data: {terminal}\n\n")))
            .unwrap())
    }
}

#[tokio::test]
async fn org_compaction_full_access_and_auto_resume_keep_exact_consent_without_tool_grant() {
    for setting in [AccessMode::Autopilot, AccessMode::Nyxmode] {
        let mut f = orchestrator_fixture("org_compact_runtime").await;
        mode(&f, setting).await;
        let owner = org(&f).await;
        let catalog = catalog(&f).await;
        let provider = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/responses/compact"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"output":[]})))
            .mount(&provider)
            .await;
        compact_connection(&f, &owner, &catalog, "compact-org", &provider.uri()).await;
        grant(&f, &Uuid::new_v4().to_string()).await;
        let body = json!({"model":"m","input":"organization account context"});
        let initial = journeys::execute(
            &f,
            &f.row,
            &CompactEgress {
                fixture: &f,
                body: body.clone(),
            },
        )
        .await
        .unwrap();
        assert!(initial.error.is_none(), "{:?}", initial.error);
        if setting == AccessMode::Autopilot {
            assert_eq!(initial.text, "Compacted reply");
        } else {
            assert!(
                initial.text.contains("confirmation card"),
                "{}",
                initial.text
            );
            assert!(provider.received_requests().await.unwrap().is_empty());
            let card = f
                .state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .find_one(doc! {"conversation_id":&f.row.id,"status":"pending",
                "tool_name":"nyxid.service_execution"})
                .await
                .unwrap()
                .unwrap();
            let acknowledgement = card.get_str("_id").unwrap().to_owned();
            idle(&mut f).await;
            acks::decide(&f.state.db, &f.owner, &f.row.id, &acknowledgement, true)
                .await
                .unwrap();
            let resumed = engine::begin_turn(
                &f.state.db,
                &f.owner,
                &engine::TurnRequest {
                    acknowledgement_id: Some(acknowledgement.clone()),
                    retry_of: None,
                    attachment_ids: vec![],
                    agent_id: None,
                    conversation_id: Some(f.row.id.clone()),
                    text: "Continue".into(),
                    model: None,
                    access_mode: None,
                },
                &f.state.encryption_keys,
            )
            .await
            .unwrap();
            let result = journeys::execute(
                &f,
                &resumed,
                &CompactEgress {
                    fixture: &f,
                    body: body.clone(),
                },
            )
            .await
            .unwrap();
            assert!(result.error.is_none(), "{:?}", result.error);
            assert_eq!(result.text, "Compacted reply");
            journeys::settled(&f, &resumed, &result).await;
            let next = engine::begin_turn(
                &f.state.db,
                &f.owner,
                &engine::TurnRequest {
                    acknowledgement_id: None,
                    retry_of: None,
                    attachment_ids: vec![],
                    agent_id: None,
                    conversation_id: Some(f.row.id.clone()),
                    text: "Another turn".into(),
                    model: None,
                    access_mode: None,
                },
                &f.state.encryption_keys,
            )
            .await
            .unwrap();
            let result = journeys::execute(
                &f,
                &next,
                &CompactEgress {
                    fixture: &f,
                    body: body.clone(),
                },
            )
            .await
            .unwrap();
            assert!(result.error.is_none(), "{:?}", result.error);
            assert!(result.text.contains("confirmation card"), "{}", result.text);
            let (status, refusal) =
                compact_call(&f, "compact-org", &body, Some(&acknowledgement)).await;
            assert_eq!(status, 409, "{refusal}");
            assert_eq!(refusal["error"], "acknowledgement_invalid", "{refusal}");
        }
        assert_eq!(provider.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            f.state
                .db
                .collection::<Document>(COLLECTION)
                .count_documents(doc! {"kind":"model_consent"})
                .await
                .unwrap(),
            0
        );
    }
}
