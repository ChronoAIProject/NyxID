use super::*;
use crate::{
    models::{assistant_settings::AccessMode, org_membership::OrgRole, user::UserType},
    services::{
        assistant_acknowledgement_service as acks,
        assistant_authority_tests::{Fixture, connected, orchestrator_fixture},
        assistant_nyxagent as engine,
        machine_integration_tests::{node, peer},
    },
    test_utils,
};
use bson::Document;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

async fn set_mode(f: &Fixture, mode: AccessMode) {
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(mode),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}
async fn signed_call(
    f: &Fixture,
    node: &crate::models::node::Node,
    job: &str,
    slug: &str,
) -> AppResult<Response> {
    let row = f
        .state
        .db
        .collection::<Document>(JOBS)
        .find_one(doc! {"_id":job})
        .await?
        .unwrap();
    let mut parameters = json!({"job_id":job,"conversation_id":f.row.id,"runtime_id":node.machine.as_ref().unwrap().runtime_id,
        "method":"POST","path":format!("/s/{slug}/upload"),"headers":[["content-type","application/octet-stream"]]});
    if let Ok(authority) = row.get_document("machine_authority") {
        parameters["authority_binding"] = json!({"context_id":authority.get_str("context_id").unwrap(),"revision":authority.get_i64("revision").unwrap(),"lease_id":authority.get_str("lease_id").unwrap()});
    }
    let mut request = nyxid_machine::Request {
        version: 1,
        authority: None,
        external_authority: None,
        request_id: uuid::Uuid::new_v4().to_string(),
        node_id: node.id.clone(),
        operation: nyxid_machine::Operation::ServiceCall,
        parameters,
        timestamp: chrono::Utc::now().timestamp(),
        nonce: uuid::Uuid::new_v4().to_string(),
        signature: String::new(),
    };
    let secret =
        node_service::get_node_signing_secret(&f.state.db, &f.state.encryption_keys, &node.id)
            .await?;
    request.signature = nyxid_machine::signing::sign(&request, &secret);
    let (sender, _receiver) = mpsc::channel(4);
    let session = Session::new(sender);
    let (upload, receiver) = mpsc::channel(2);
    upload
        .send(Ok(b"dynamic job upload".to_vec()))
        .await
        .unwrap();
    upload.send(Ok(Vec::new())).await.unwrap();
    drop(upload);
    Box::pin(authorize_and_execute(
        &f.state, &session, &node.id, request, receiver,
    ))
    .await
}

#[tokio::test]
async fn actual_job_launch_detached_gateway_and_revocation_preserve_personal_platform_and_org_contracts()
 {
    for kind in ["personal", "platform", "org"] {
        let f = orchestrator_fixture("job_service_policy_journey").await;
        let n = node(&f, &f.owner).await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("uploaded"))
            .mount(&server)
            .await;
        let owner = if kind == "org" {
            let org = uuid::Uuid::new_v4().to_string();
            f.state
                .db
                .collection("users")
                .insert_one(test_utils::test_user(&org, UserType::Org))
                .await
                .unwrap();
            f.state
                .db
                .collection("org_memberships")
                .insert_one(test_utils::test_membership(
                    &org,
                    &f.owner,
                    OrgRole::Admin,
                    None,
                ))
                .await
                .unwrap();
            org
        } else {
            f.owner.clone()
        };
        let slug = format!("job-{kind}");
        let id = if kind == "platform" {
            let mut catalog = test_utils::test_auto_connected_catalog_service();
            catalog.slug = slug.clone();
            catalog.base_url = server.uri();
            catalog.auth_method = "bearer".into();
            catalog.auth_key_name = "Authorization".into();
            catalog.platform_key = Some(crate::models::downstream_service::PlatformKeyConfig {
                enabled: true,
                audience: crate::models::downstream_service::PlatformKeyAudience::Public,
                allowed_owner_ids: vec![],
            });
            catalog.credential_encrypted = f
                .state
                .encryption_keys
                .encrypt(b"platform-test-credential")
                .await
                .unwrap();
            assert!(crate::services::platform_key_service::has_platform_key(
                &catalog
            ));
            let id = catalog.id.clone();
            f.state
                .db
                .collection("downstream_services")
                .insert_one(catalog)
                .await
                .unwrap();
            id
        } else {
            connected(&f.state.db, &owner, &slug, &server.uri()).await
        };
        let grant = if kind == "platform" {
            doc! {"$addToSet":{"allowed_platform_service_ids":&id}}
        } else {
            doc! {"$addToSet":{"allowed_service_ids":&id}}
        };
        f.state
            .db
            .collection::<Document>("api_keys")
            .update_one(doc! {"_id":&f.chat.api_key_id}, grant)
            .await
            .unwrap();
        set_mode(
            &f,
            if kind == "org" {
                AccessMode::Nyxmode
            } else {
                AccessMode::Autopilot
            },
        )
        .await;
        let chat = acks::for_key(&f.state.db, &f.owner, Some(&f.chat.api_key_id))
            .await
            .unwrap()
            .unwrap();
        let (task, mut requests) = peer(&f, &n, json!({"status":"running"})).await;
        let mut arguments =
            json!({"machine":n.id,"command":"consume service","services":[id],"timeout_secs":120});
        let mut result = Box::pin(crate::handlers::machine_tools::call(
            &f.state,
            &chat,
            "nyx__machine_exec",
            arguments.clone(),
        ))
        .await
        .unwrap_or_else(|error| panic!("{kind} job launch: {error:?}"));
        if kind == "org" {
            assert_eq!(result["error"], "acknowledgement_required");
            assert!(requests.try_recv().is_err());
            let ack = result["acknowledgement_id"].as_str().unwrap();
            let card = f
                .state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .find_one(doc! {"_id":ack})
                .await
                .unwrap()
                .unwrap();
            assert!(
                card.get_str("summary")
                    .unwrap()
                    .contains("organization connection")
            );
            acks::decide(&f.state.db, &f.owner, &f.row.id, ack, true)
                .await
                .unwrap();
            arguments["acknowledgement_id"] = json!(ack);
            result = Box::pin(crate::handlers::machine_tools::call(
                &f.state,
                &chat,
                "nyx__machine_exec",
                arguments.clone(),
            ))
            .await
            .unwrap();
        }
        assert_eq!(result["status"], "running", "{kind}: {result}");
        let issued = requests.recv().await.unwrap();
        let job = issued.parameters["job_id"].as_str().unwrap().to_owned();
        if kind != "org" {
            Box::pin(crate::handlers::machine_tools::call(
                &f.state,
                &chat,
                "nyx__machine_exec",
                arguments,
            ))
            .await
            .unwrap();
            let second = requests.recv().await.unwrap();
            assert_ne!(issued.parameters["job_id"], second.parameters["job_id"]);
        }
        engine::finish_turn(
            &f.state.db,
            &f.row,
            &f.chat.api_key_id,
            &uuid::Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: "launched".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        )
        .await
        .unwrap();
        let response = signed_call(&f, &n, &job, &slug).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        if kind == "org" {
            f.state
                .db
                .collection::<Document>("user_services")
                .update_one(doc! {"_id":&id}, doc! {"$inc":{"state_version":1_i64}})
                .await
                .unwrap();
        } else if kind == "personal" {
            f.state
                .db
                .collection::<Document>("api_keys")
                .update_one(
                    doc! {"_id":&f.chat.api_key_id},
                    doc! {"$set":{"is_active":false}},
                )
                .await
                .unwrap();
        } else {
            let target = Box::pin(crate::services::proxy_service::resolve_proxy_target(
                &f.state.db,
                &f.state.encryption_keys,
                &f.owner,
                &id,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
            ))
            .await
            .unwrap();
            let persisted = f
                .state
                .db
                .collection::<crate::models::machine_job::MachineJob>(JOBS)
                .find_one(doc! {"_id":&job})
                .await
                .unwrap()
                .unwrap();
            let ingress = crate::services::machine_gateway_service::Ingress {
                declared_id: id.clone(),
                git: None,
                job: Some((&persisted).into()),
            };
            let held = crate::services::assistant_service_execution::HttpEgressGuard::for_proxy(
                None,
                Some(&ingress),
                &f.state.db,
                &f.state.encryption_keys,
                &target,
                "POST",
                "/upload",
            )
            .expect("catalog-only assistant job must have a final egress guard");
            held.revalidate().await.unwrap();
            f.state.db.collection::<Document>(JOBS).update_one(doc! {"_id":&job},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
            assert!(
                held.revalidate().await.is_err(),
                "held catalog authority must expire before send"
            );
        }
        assert!(signed_call(&f, &n, &job, &slug).await.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        task.abort();
    }
}

#[tokio::test]
async fn job_plan_concurrent_claim_has_one_winner_and_fresh_identical_invocation_gets_new_id() {
    let f = orchestrator_fixture("job_plan_claim").await;
    let args = json!({"command":"pwd"});
    let plan =
        crate::services::assistant_job_service_use::prepare(&f.state, &f.chat, "node", &args, &[])
            .await
            .unwrap()
            .unwrap_or_else(|v| panic!("{v}"));
    let (a, b) = tokio::join!(plan.claim(&f.state.db), plan.claim(&f.state.db));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let next =
        crate::services::assistant_job_service_use::prepare(&f.state, &f.chat, "node", &args, &[])
            .await
            .unwrap()
            .unwrap_or_else(|v| panic!("{v}"));
    assert_ne!(plan.id, next.id);
}
