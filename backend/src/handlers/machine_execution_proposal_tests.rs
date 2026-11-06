//! Signed ingress + real-Mongo proposal/decision/send/receipt regressions.
use super::*;
use crate::models::{approval_request::ApprovalRequest, node::Node};
use crate::services::{
    assistant_acknowledgement_service as acks,
    assistant_authority_tests::{Fixture, connected, orchestrator_fixture},
    execution_permission_dispatch::{phase_b_tests as permissions, proposals},
    machine_integration_tests::{node, peer},
};
use bson::Document;
use futures::TryStreamExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

struct Setup {
    f: Fixture,
    node: Node,
    job: String,
    connection: String,
    server: MockServer,
    peer: tokio::task::JoinHandle<()>,
}
impl Setup {
    async fn new(negotiated: bool) -> Self {
        let f = orchestrator_fixture("machine_proposal_b2").await;
        permissions::enroll_global(&f.state.db, &f.owner).await;
        permissions::task(&f.state.db, &f.row).await;
        permissions::legacy_review(&f.state, &f.owner).await;
        let mut node = node(&f, &f.owner).await;
        node.machine.as_mut().unwrap().authority_versions = vec![2];
        f.state
            .db
            .collection::<Document>("nodes")
            .update_one(
                doc! {"_id":&node.id},
                doc! {"$set":{"machine":bson::to_bson(&node.machine).unwrap()}},
            )
            .await
            .unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_string("actual provider response"))
            .mount(&server)
            .await;
        let connection = connected(&f.state.db, &f.owner, "b2-test", &server.uri()).await;
        let chat = acks::for_key(&f.state.db, &f.owner, Some(&f.chat.api_key_id))
            .await
            .unwrap()
            .unwrap();
        let (peer, mut requests) = peer(&f, &node, json!({"status":"running"})).await;
        let caps = serde_json::from_value(
            json!({"machine":node.machine,"machine_execution_approval_v1":negotiated}),
        )
        .unwrap();
        f.state.node_ws_manager.record_capabilities(&node.id, &caps);
        f.state
            .node_ws_manager
            .mark_status_update_received(&node.id);
        f.state.db.collection::<Document>("nodes").update_one(doc!{"_id":&node.id},doc!{"$set":{"connection_owner.capabilities_resolved":true,"connection_owner.machine_execution_approval_v1":negotiated}}).await.unwrap();
        let mut args = json!({"machine":node.id,"command":"consume one service","services":[connection],"timeout_secs":120});
        let mut result = crate::handlers::machine_tools::call(
            &f.state,
            &chat,
            "nyx__machine_exec",
            args.clone(),
        )
        .await
        .unwrap();
        if let Some(card) = result["acknowledgement_id"].as_str() {
            acks::decide(&f.state.db, &f.owner, &f.row.id, card, true)
                .await
                .unwrap();
            args["acknowledgement_id"] = json!(card);
            result =
                crate::handlers::machine_tools::call(&f.state, &chat, "nyx__machine_exec", args)
                    .await
                    .unwrap();
        }
        assert_eq!(result["status"], "running", "{result}");
        let issued = requests.recv().await.unwrap();
        let job = issued.parameters["job_id"].as_str().unwrap().to_owned();
        assert!(issued.authority.is_some());
        f.state.db.collection::<Document>("service_approval_configs").insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.owner,"service_id":&connection,"service_name":"Machine test","approval_required":true,"approval_mode":"per_request","rules":[],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        f.state
            .db
            .collection::<Document>("notification_channels")
            .update_one(
                doc! {"user_id":&f.owner},
                doc! {"$set":{"approval_timeout_secs":60}},
            )
            .await
            .unwrap();
        Self {
            f,
            node,
            job,
            connection,
            server,
            peer,
        }
    }
    async fn request(
        &self,
        mode: Option<&str>,
        seconds: i64,
        path: &str,
    ) -> nyxid_machine::Request {
        let row = self
            .f
            .state
            .db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id":&self.job})
            .await
            .unwrap()
            .unwrap();
        let a = row.get_document("machine_authority").unwrap();
        let mut p = json!({"job_id":self.job,"conversation_id":self.f.row.id,"runtime_id":self.node.machine.as_ref().unwrap().runtime_id,
            "authority_binding":{"context_id":a.get_str("context_id").unwrap(),"revision":a.get_i64("revision").unwrap(),"lease_id":a.get_str("lease_id").unwrap()},
            "method":if path.starts_with("/_nyxid/"){"GET"}else{"POST"},"path":path,
            "headers":[["content-type","application/json"],["idempotency-key","same-downstream-key"],["prefer","respond-async"]]});
        if let Some(mode) = mode {
            p["execution_approval"] = json!({"version":1,"mode":mode,"waiter_until_ms":(chrono::Utc::now()+chrono::Duration::seconds(seconds)).timestamp_millis()});
        }
        let mut r = nyxid_machine::Request {
            version: 1,
            authority: None,
            external_authority: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            node_id: self.node.id.clone(),
            operation: nyxid_machine::Operation::ServiceCall,
            parameters: p,
            timestamp: chrono::Utc::now().timestamp(),
            nonce: uuid::Uuid::new_v4().to_string(),
            signature: String::new(),
        };
        let secret = node_service::get_node_signing_secret(
            &self.f.state.db,
            &self.f.state.encryption_keys,
            &self.node.id,
        )
        .await
        .unwrap();
        r.signature = nyxid_machine::signing::sign(&r, &secret);
        r
    }
    async fn start(
        &self,
        r: nyxid_machine::Request,
    ) -> (
        tokio::task::JoinHandle<AppResult<Response>>,
        tokio_util::sync::CancellationToken,
    ) {
        let (sender, _receiver) = mpsc::channel(4);
        let session = Session::new(sender);
        let token = session.closed.child_token();
        session.calls.lock().await.insert(
            uuid::Uuid::parse_str(&r.request_id).unwrap(),
            ActiveCall {
                job_id: self.job.clone(),
                cancel: token.clone(),
            },
        );
        let (upload, receiver) = mpsc::channel(2);
        upload
            .send(Ok(br#"{"message":"hello"}"#.to_vec()))
            .await
            .unwrap();
        upload.send(Ok(Vec::new())).await.unwrap();
        drop(upload);
        let state = self.f.state.clone();
        let node = self.node.id.clone();
        (
            tokio::spawn(async move {
                Box::pin(authorize_and_execute(&state, &session, &node, r, receiver)).await
            }),
            token,
        )
    }
    async fn proposal(&self) -> ApprovalRequest {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Some(row) = self
                    .f
                    .state
                    .db
                    .collection::<ApprovalRequest>("approval_requests")
                    .find_one(doc! {"execution_proposal.version":2,"execution_proposal.v2.notification_claimed_at":{"$ne":null}})
                    .await
                    .unwrap()
                {
                    return row;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("proposal created")
    }
    async fn approve(&self, row: &ApprovalRequest) -> AppResult<ApprovalRequest> {
        let p = row
            .execution_proposal
            .as_ref()
            .unwrap()
            .v2
            .as_ref()
            .unwrap();
        let c = p.choices.iter().find(|c| c.id == "operation").unwrap();
        proposals::decide(
            &self.f.state,
            &self.f.owner,
            &row.id,
            &proposals::Decision {
                approved: true,
                expected_revision: p.revision,
                choice_id: Some(c.id.clone()),
                choice_digest: Some(c.digest.clone()),
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
    }
    async fn cleanup(self) {
        self.peer.abort();
        self.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn machine_proposal_connected_approval_returns_actual_response() {
    let s = Setup::new(true).await;
    let r = s.request(Some("connected"), 100, "/s/b2-test/upload").await;
    let (call, _) = s.start(r).await;
    let row = s.proposal().await;
    assert!(!call.is_finished());
    assert!(s.server.received_requests().await.unwrap().is_empty());
    let p = row
        .execution_proposal
        .as_ref()
        .unwrap()
        .v2
        .as_ref()
        .unwrap();
    assert!(!p.background_allowed);
    assert!(p.notification_claimed_at.is_some());
    assert!(
        proposals::v2::execute(&s.f.state, &row)
            .await
            .unwrap()
            .is_none()
    );
    s.approve(&row).await.unwrap();
    let response = call.await.unwrap().unwrap();
    assert_eq!(response.status(), 201);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap(),
        "actual provider response"
    );
    assert_eq!(s.server.received_requests().await.unwrap().len(), 1);
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_old_node_legacy_gap_matches_main() {
    let s = Setup::new(false).await;
    s.f.state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"user_id":&s.f.owner},
            doc! {"$set":{"approval_timeout_secs":1}},
        )
        .await
        .unwrap();
    let baseline =
        s.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap();
    let (call, _) = s
        .start(s.request(None, 100, "/s/b2-test/upload").await)
        .await;
    let error = call.await.unwrap().unwrap_err();
    assert!(
        matches!(&error,AppError::ApprovalFailed{reason,approve_url,..} if reason=="Approval request expired" && approve_url.starts_with("http")),
        "{error:?}"
    );
    let rows =
        s.f.state
            .db
            .collection::<Document>("approval_requests")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get_document("execution_proposal").is_err());
    assert_eq!(rows[0].get_str("approval_mode"), Ok("per_request"));
    assert_eq!(
        s.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        baseline
    );
    assert!(s.server.received_requests().await.unwrap().is_empty());
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_short_window_times_out_without_card() {
    let s = Setup::new(true).await;
    let (call, _) = s
        .start(s.request(Some("connected"), 14, "/s/b2-test/upload").await)
        .await;
    assert!(
        matches!(call.await.unwrap(),Err(AppError::ApprovalFailed{reason,..}) if reason=="Approval request timed out")
    );
    assert_eq!(
        s.f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(s.server.received_requests().await.unwrap().is_empty());
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_readiness_loss_keeps_fresh_gaps_legacy() {
    let s = Setup::new(true).await;
    // Remove only the Machine reader capability. A duplicate generation would
    // correctly block every execution gate, which is a different scenario.
    let changed = s.f.state.db.collection::<Document>("security_replicas")
        .update_many(doc! {}, doc! {"$pull": {"capabilities": crate::services::execution_permission_dispatch::machine::READINESS}})
        .await.unwrap();
    assert_eq!(changed.matched_count, 1);
    assert!(
        crate::services::security_readiness_service::report(
            &s.f.state.db,
            "permissions:execution-authoring-v2"
        )
        .await
        .unwrap()
        .ready
    );
    assert!(
        !crate::services::security_readiness_service::report(
            &s.f.state.db,
            crate::services::execution_permission_dispatch::machine::READINESS
        )
        .await
        .unwrap()
        .ready
    );
    s.f.state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"user_id": &s.f.owner},
            doc! {"$set": {"approval_timeout_secs": 1}},
        )
        .await
        .unwrap();
    let (call, _) = s
        .start(s.request(Some("connected"), 100, "/s/b2-test/upload").await)
        .await;
    let error = call.await.unwrap().unwrap_err();
    assert!(
        matches!(&error, AppError::ApprovalFailed { reason, .. } if reason == "Approval request expired"),
        "{error}"
    );
    let rows =
        s.f.state
            .db
            .collection::<Document>("approval_requests")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get_document("execution_proposal").is_err());
    assert!(s.server.received_requests().await.unwrap().is_empty());
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_unmarked_negotiation_adds_zero_database_commands() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    let mut s = Setup::new(true).await;
    // Model a job admitted without execution-permission lineage. Its ordinary
    // service request must remain identical when node negotiation is present.
    s.f.state
        .db
        .collection::<Document>(JOBS)
        .update_one(
            doc! {"_id": &s.job},
            doc! {"$unset": {"execution_task": ""}},
        )
        .await
        .unwrap();
    s.f.state
        .db
        .collection::<Document>("service_approval_configs")
        .update_many(doc! {}, doc! {"$set": {"approval_required": false}})
        .await
        .unwrap();
    s.f.state
        .db
        .collection::<Document>("notification_channels")
        .update_many(doc! {}, doc! {"$set": {"approval_required": false}})
        .await
        .unwrap();
    let job =
        s.f.state
            .db
            .collection::<MachineJob>(JOBS)
            .find_one(doc! {"_id": &s.job})
            .await
            .unwrap()
            .unwrap();
    assert!(
        crate::services::machine_gateway_service::job_auth(&s.f.state.db, &job)
            .await
            .unwrap()
            .execution_permission
            .is_none()
    );
    let capturing = Arc::new(AtomicBool::new(false));
    let commands = Arc::new(Mutex::new(Vec::<String>::new()));
    let active = capturing.clone();
    let captured = commands.clone();
    let database = s.f.state.db.name().to_owned();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == database
            && active.load(Ordering::Relaxed)
        {
            captured.lock().unwrap().push(event.command_name);
        }
    }));
    s.f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(s.f.state.db.name());
    let mut samples = Vec::new();
    for mode in [None, Some("connected"), None, Some("connected")] {
        let request = s.request(mode, 100, "/s/b2-test/upload").await;
        commands.lock().unwrap().clear();
        capturing.store(true, Ordering::Relaxed);
        let (call, _) = s.start(request).await;
        let response = call.await.unwrap().unwrap();
        assert_eq!(response.status(), 201);
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        capturing.store(false, Ordering::Relaxed);
        let mut sample = commands.lock().unwrap().clone();
        sample.sort();
        samples.push(sample);
    }
    // Compare warmed calls, including every command (writes/transactions too).
    assert!(!samples[2].is_empty());
    assert_eq!(samples[2], samples[3]);
    assert_eq!(
        s.f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(s.server.received_requests().await.unwrap().len(), 4);
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_disconnect_is_unsent_and_retry_is_fresh() {
    let s = Setup::new(true).await;
    let (call, token) = s
        .start(s.request(Some("connected"), 100, "/s/b2-test/upload").await)
        .await;
    let row = s.proposal().await;
    token.cancel();
    assert!(matches!(
        call.await.unwrap(),
        Err(AppError::ClientDisconnected)
    ));
    assert!(s.approve(&row).await.is_err());
    assert!(
        proposals::v2::execute(&s.f.state, &row)
            .await
            .unwrap()
            .is_none()
    );
    let (retry, token) = s
        .start(s.request(Some("connected"), 100, "/s/b2-test/upload").await)
        .await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if s.f
                .state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {})
                .await
                .unwrap()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    token.cancel();
    assert!(retry.await.unwrap().is_err());
    assert!(s.server.received_requests().await.unwrap().is_empty());
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_async_dedupe_signed_readback_and_job_end() {
    let s = Setup::new(true).await;
    let mut calls = Vec::new();
    for _ in 0..20 {
        calls.push(
            s.start(s.request(Some("async"), 1, "/s/b2-test/upload").await)
                .await
                .0,
        );
    }
    let row = s.proposal().await;
    for c in calls {
        assert!(matches!(
            c.await.unwrap(),
            Err(AppError::ApprovalRequired { .. })
        ));
    }
    assert_eq!(
        s.f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let approved = s.approve(&row).await.unwrap();
    if let Some(response) = proposals::v2::execute(&s.f.state, &approved)
        .await
        .unwrap_or_else(|error| panic!("Machine replay failed: {error}"))
    {
        let _ = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
    }
    assert!(
        proposals::v2::execute(&s.f.state, &approved)
            .await
            .unwrap()
            .is_none()
    );
    let path = format!("/_nyxid/approvals/{}/status", row.id);
    let (poll, _) = s
        .start(s.request(Some("connected"), 100, &path).await)
        .await;
    let response = poll.await.unwrap().unwrap();
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["execution"]["result"]["status"], 201, "{result}");
    assert_eq!(s.server.received_requests().await.unwrap().len(), 1);
    crate::services::machine_service::finish(&s.f.state.db, &s.job)
        .await
        .unwrap();
    let (poll, _) = s
        .start(s.request(Some("connected"), 100, &path).await)
        .await;
    assert!(poll.await.unwrap().is_err());
    assert_eq!(s.server.received_requests().await.unwrap().len(), 1);
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_source_revocation_blocks_decision_and_send() {
    for change in [
        "finished",
        "lease",
        "actor",
        "task",
        "revision",
        "runtime",
        "key",
        "owner_control",
        "node_offline",
    ] {
        let s = Setup::new(true).await;
        let (call, _) = s
            .start(s.request(Some("async"), 100, "/s/b2-test/upload").await)
            .await;
        assert!(matches!(
            call.await.unwrap(),
            Err(AppError::ApprovalRequired { .. })
        ));
        let row = s.proposal().await;
        let approved = if change == "finished" {
            Some(s.approve(&row).await.unwrap())
        } else {
            None
        };
        match change {
            "finished" => crate::services::machine_service::finish(&s.f.state.db, &s.job)
                .await
                .unwrap(),
            "lease" => {
                s.f.state
                    .db
                    .collection::<Document>("machine_authority_leases")
                    .update_many(
                        doc! {"job_id":&s.job},
                        doc! {"$set":{"authority.expires_at_ms":0_i64}},
                    )
                    .await
                    .unwrap();
            }
            "actor" => {
                s.f.state
                    .db
                    .collection::<Document>(JOBS)
                    .update_one(
                        doc! {"_id":&s.job},
                        doc! {"$set":{"user_id":uuid::Uuid::new_v4().to_string()}},
                    )
                    .await
                    .unwrap();
            }
            "task" => {
                s.f.state
                    .db
                    .collection::<Document>("assistant_messages")
                    .update_many(
                        doc! {"conversation_id":&s.f.row.id,"execution_task":{"$ne":null}},
                        doc! {"$set":{"execution_task.cancelled":true}},
                    )
                    .await
                    .unwrap();
            }
            "revision" => {
                s.f.state.db.collection::<Document>("assistant_agents").update_one(doc!{"_id":&s.f.chat.agent_id},doc!{"$inc":{format!("machine_access.assignments.{}.revision",s.node.id):1_i64}}).await.unwrap();
            }
            "runtime" => {
                s.f.state
                    .db
                    .collection::<Document>("nodes")
                    .update_one(
                        doc! {"_id": &s.node.id},
                        doc! {"$set": {"machine.runtime_id": "replaced-runtime"}},
                    )
                    .await
                    .unwrap();
            }
            "node_offline" => {
                s.f.state
                    .db
                    .collection::<Document>("nodes")
                    .update_one(
                        doc! {"_id": &s.node.id},
                        doc! {"$set": {"status": "offline"}},
                    )
                    .await
                    .unwrap();
            }
            "key" => {
                s.f.state
                    .db
                    .collection::<Document>("api_keys")
                    .update_one(
                        doc! {"_id": &s.f.chat.api_key_id},
                        doc! {"$set": {"is_active": false}},
                    )
                    .await
                    .unwrap();
            }
            "owner_control" => {
                let id = crate::services::machine_desktop_service::desktop_id(
                    &s.node.id,
                    None,
                    nyxid_machine::desktop::Display::Secure,
                );
                s.f.state.db.collection::<Document>("machine_desktops").insert_one(doc! {"_id": id, "node_id": &s.node.id, "display": "secure", "status": "owner"}).await.unwrap();
            }
            _ => unreachable!(),
        }
        if let Some(approved) = approved {
            assert!(proposals::v2::execute(&s.f.state, &approved).await.is_err());
        } else {
            assert!(s.approve(&row).await.is_err(), "{change}");
        }
        assert!(s.server.received_requests().await.unwrap().is_empty());
        s.cleanup().await;
    }
}

#[tokio::test]
async fn machine_proposal_delivery_checks_are_bounded_and_non_b2_adds_no_commands() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let mut s = Setup::new(true).await;
    s.server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 3 * 1024 * 1024 + 1]))
        .mount(&s.server)
        .await;
    let database = s.f.state.db.name().to_owned();
    let commands = Arc::new(AtomicUsize::new(0));
    let aggregates = Arc::new(AtomicUsize::new(0));
    let c = commands.clone();
    let a = aggregates.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == database
        {
            c.fetch_add(1, Ordering::Relaxed);
            if event.command.get_str("aggregate") == Ok("machine_jobs")
                && event.command.get_array("pipeline").is_ok_and(|pipeline| {
                    pipeline.iter().any(|stage| {
                        stage
                            .as_document()
                            .and_then(|s| s.get_document("$lookup").ok())
                            .is_some_and(|lookup| lookup.get_str("as") == Ok("enrollment"))
                    })
                })
            {
                a.fetch_add(1, Ordering::Relaxed);
            }
        }
    }));
    s.f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(s.f.state.db.name());
    // A legacy or unmarked response must return without ANY database command.
    commands.store(0, Ordering::Relaxed);
    let response = crate::services::execution_permission_dispatch::machine::delivery(
        &s.f.state,
        &s.f.auth,
        Response::new(Body::from("legacy")),
    )
    .await
    .unwrap();
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap(),
        "legacy"
    );
    assert_eq!(commands.load(Ordering::Relaxed), 0);
    let (call, _) = s
        .start(s.request(Some("connected"), 100, "/s/b2-test/upload").await)
        .await;
    let row = s.proposal().await;
    s.approve(&row).await.unwrap();
    let response = call.await.unwrap().unwrap();
    assert_eq!(
        aggregates.load(Ordering::Relaxed),
        1,
        "one before-header fence"
    );
    let start = std::time::Instant::now();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body.len(), 3 * 1024 * 1024 + 1);
    let checks = aggregates.load(Ordering::Relaxed);
    assert!(checks >= 4);
    assert!(
        checks <= 4 + start.elapsed().as_secs() as usize,
        "{checks} checks exceeds byte/time cadence"
    );
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_stream_stops_after_live_revocation_without_recall() {
    use futures::StreamExt;
    for loss in ["job_finished", "enrollment_disabled", "actor_deleting"] {
        let s = Setup::new(true).await;
        s.server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 2 * 1024 * 1024]))
            .mount(&s.server)
            .await;
        let (call, _) = s
            .start(s.request(Some("connected"), 100, "/s/b2-test/upload").await)
            .await;
        let row = s.proposal().await;
        s.approve(&row).await.unwrap();
        let mut body = call.await.unwrap().unwrap().into_body().into_data_stream();
        let first = body.next().await.unwrap().unwrap();
        assert!(!first.is_empty());
        match loss {
            "job_finished" => crate::services::machine_service::finish(&s.f.state.db, &s.job)
                .await
                .unwrap(),
            "enrollment_disabled" => {
                s.f.state
                    .db
                    .collection::<Document>("feature_flag_overrides")
                    .update_many(
                        doc! {"flag_key": "permissions:execution-v1"},
                        doc! {"$set": {"enabled": false}},
                    )
                    .await
                    .unwrap();
            }
            "actor_deleting" => {
                s.f.state
                    .db
                    .collection::<Document>("users")
                    .update_one(
                        doc! {"_id": &s.f.owner},
                        doc! {"$set": {"platform_personal_deleting": true}},
                    )
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        tokio::time::sleep(Duration::from_millis(1050)).await;
        assert!(body.next().await.unwrap().is_err(), "{loss}");
        assert_eq!(s.server.received_requests().await.unwrap().len(), 1);
        s.cleanup().await;
    }
}

#[tokio::test]
async fn machine_proposal_connected_timeout_is_terminal_and_worker_cannot_send() {
    let s = Setup::new(true).await;
    let (call, _) = s
        .start(s.request(Some("connected"), 25, "/s/b2-test/upload").await)
        .await;
    let row = s.proposal().await;
    let error = tokio::time::timeout(Duration::from_secs(30), call)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(error,AppError::ApprovalFailed{reason,..} if reason=="Approval request timed out")
    );
    assert!(s.approve(&row).await.is_err());
    assert!(
        proposals::v2::execute(&s.f.state, &row)
            .await
            .unwrap()
            .is_none()
    );
    assert!(s.server.received_requests().await.unwrap().is_empty());
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_cancel_after_claim_before_handoff_is_not_sent() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    let mut s = Setup::new(true).await;
    let barrier = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(Mutex::new(None::<tokio_util::sync::CancellationToken>));
    let token = cancelled.clone();
    let armed = barrier.clone();
    let database = s.f.state.db.name().to_owned();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| match event {
        CommandEvent::Started(event) if event.db == database => {
            if event.command.get_array("updates").is_ok_and(|updates| {
                updates.iter().any(|update| {
                    update
                        .as_document()
                        .and_then(|u| u.get_document("u").ok())
                        .and_then(|u| u.get_document("$set").ok())
                        .is_some_and(|set| {
                            set.get_str("execution_proposal.v2.state") == Ok("dispatched")
                        })
                })
            }) {
                armed.store(true, Ordering::Relaxed);
            }
        }
        CommandEvent::Succeeded(event)
            if event.command_name == "commitTransaction" && armed.load(Ordering::Relaxed) =>
        {
            if let Some(token) = token.lock().unwrap().as_ref() {
                token.cancel();
            }
        }
        _ => {}
    }));
    s.f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(s.f.state.db.name());
    let (call, token) = s
        .start(s.request(Some("connected"), 100, "/s/b2-test/upload").await)
        .await;
    *cancelled.lock().unwrap() = Some(token);
    let row = s.proposal().await;
    s.approve(&row).await.unwrap();
    assert!(matches!(
        call.await.unwrap(),
        Err(AppError::ClientDisconnected)
    ));
    assert!(
        barrier.load(Ordering::Relaxed),
        "cancellation occurred after the claim committed"
    );
    let row =
        s.f.state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        row.execution_proposal
            .as_ref()
            .unwrap()
            .v2
            .as_ref()
            .unwrap()
            .state,
        "not_sent"
    );
    assert!(
        proposals::v2::execute(&s.f.state, &row)
            .await
            .unwrap()
            .is_none()
    );
    assert!(s.server.received_requests().await.unwrap().is_empty());
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_negotiated_covered_and_prohibited_have_no_cards() {
    for prohibited in [false, true] {
        let s = Setup::new(true).await;
        let job =
            s.f.state
                .db
                .collection::<MachineJob>(JOBS)
                .find_one(doc! {"_id":&s.job})
                .await
                .unwrap()
                .unwrap();
        let auth = crate::services::machine_gateway_service::job_auth(&s.f.state.db, &job)
            .await
            .unwrap();
        let mut grant =
            permissions::grant(&s.f.state, &auth, &s.connection, "POST", "/upload").await;
        grant
            .execution
            .as_mut()
            .unwrap()
            .v1
            .as_mut()
            .unwrap()
            .operations
            .operations[0]
            .inputs = Some(nyxid_permissions::values::InputRules {
            body: Some(nyxid_permissions::google::JsonShape::Exact {
                value: json!({"message":"hello"}),
            }),
            ..Default::default()
        });
        if prohibited {
            grant.execution.as_mut().unwrap().version = 99;
        }
        s.f.state
            .db
            .collection("approval_grants")
            .insert_one(grant)
            .await
            .unwrap();
        let mut r = s.request(Some("connected"), 100, "/s/b2-test/upload").await;
        r.parameters["headers"] = json!([["content-type", "application/json"]]);
        let secret = node_service::get_node_signing_secret(
            &s.f.state.db,
            &s.f.state.encryption_keys,
            &s.node.id,
        )
        .await
        .unwrap();
        r.signature = nyxid_machine::signing::sign(&r, &secret);
        let (call, _) = s.start(r).await;
        let result = call.await.unwrap();
        assert_eq!(result.is_ok(), !prohibited, "{result:?}");
        assert_eq!(
            s.server.received_requests().await.unwrap().len(),
            usize::from(!prohibited)
        );
        assert_eq!(
            s.f.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        s.cleanup().await;
    }
}

#[tokio::test]
async fn machine_proposal_readback_is_signed_and_bound_to_exact_job() {
    let s = Setup::new(true).await;
    let (call, _) = s
        .start(s.request(Some("async"), 100, "/s/b2-test/upload").await)
        .await;
    assert!(matches!(
        call.await.unwrap(),
        Err(AppError::ApprovalRequired { .. })
    ));
    let row = s.proposal().await;
    let path = format!("/_nyxid/approvals/{}/status", row.id);
    let mut tampered = s.request(Some("connected"), 100, &path).await;
    tampered.parameters["method"] = json!("POST");
    let (call, _) = s.start(tampered).await;
    assert!(call.await.unwrap().is_err());
    let mut other =
        s.f.state
            .db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id":&s.job})
            .await
            .unwrap()
            .unwrap();
    let other_id = uuid::Uuid::new_v4().to_string();
    other.insert("_id", &other_id);
    s.f.state
        .db
        .collection::<Document>(JOBS)
        .insert_one(other)
        .await
        .unwrap();
    let mut r = s.request(Some("connected"), 100, &path).await;
    r.parameters["job_id"] = json!(other_id);
    let secret = node_service::get_node_signing_secret(
        &s.f.state.db,
        &s.f.state.encryption_keys,
        &s.node.id,
    )
    .await
    .unwrap();
    r.signature = nyxid_machine::signing::sign(&r, &secret);
    let (call, _) = s.start(r).await;
    assert!(matches!(call.await.unwrap(), Err(AppError::NotFound(_))));
    assert!(s.server.received_requests().await.unwrap().is_empty());
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_stale_preparer_recovers_but_uncertain_never_resends() {
    let s = Setup::new(true).await;
    let (call, _) = s
        .start(s.request(Some("async"), 100, "/s/b2-test/upload").await)
        .await;
    assert!(matches!(
        call.await.unwrap(),
        Err(AppError::ApprovalRequired { .. })
    ));
    let row = s.proposal().await;
    let approved = s.approve(&row).await.unwrap();
    s.f.state.db.collection::<Document>("approval_requests").update_one(doc!{"_id":&row.id},doc!{"$set":{"execution_proposal.v2.state":"preparing","execution_proposal.v2.claim_id":"crashed-before-send","execution_proposal.v2.claimed_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::minutes(3))}}).await.unwrap();
    if let Some(response) = proposals::v2::execute(&s.f.state, &approved)
        .await
        .unwrap_or_else(|error| panic!("Machine replay failed: {error}"))
    {
        let _ = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
    }
    assert_eq!(s.server.received_requests().await.unwrap().len(), 1);
    s.f.state.db.collection::<Document>("approval_requests").update_one(doc!{"_id":&row.id},doc!{"$set":{"execution_proposal.v2.state":"unknown","execution_proposal.v2.failure_code":"provider_outcome_unknown"}}).await.unwrap();
    assert!(
        proposals::v2::execute(&s.f.state, &approved)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(s.server.received_requests().await.unwrap().len(), 1);
    s.cleanup().await;
}

#[tokio::test]
async fn machine_proposal_detached_job_keeps_original_task_and_same_lease_renewal() {
    let s = Setup::new(true).await;
    crate::services::assistant_nyxagent::finish_turn(
        &s.f.state.db,
        &s.f.row,
        &s.f.row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &crate::services::assistant_nyxagent::TurnResult {
            text: "Job continues".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let (call, _) = s
        .start(s.request(Some("connected"), 100, "/s/b2-test/upload").await)
        .await;
    let row = s.proposal().await;
    // Model the existing independent lease renewal; approval must not do it.
    let end = chrono::Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS;
    s.f.state
        .db
        .collection::<Document>("machine_authority_leases")
        .update_one(
            doc! {"job_id":&s.job},
            doc! {"$set":{"authority.expires_at_ms":end}},
        )
        .await
        .unwrap();
    s.approve(&row).await.unwrap();
    let response = call.await.unwrap().unwrap();
    assert_eq!(response.status(), 201);
    let _ = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let lease =
        s.f.state
            .db
            .collection::<Document>("machine_authority_leases")
            .find_one(doc! {"job_id":&s.job})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        lease
            .get_document("authority")
            .unwrap()
            .get_i64("expires_at_ms")
            .unwrap(),
        end
    );
    assert_eq!(s.server.received_requests().await.unwrap().len(), 1);
    s.cleanup().await;
}
