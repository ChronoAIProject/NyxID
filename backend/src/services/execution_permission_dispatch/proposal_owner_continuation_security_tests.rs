//! Defensive owner-runner regressions for the five independent #3064 findings.
use super::*;
use crate::services::platform_manager_contract::{
    NyxAgentEgress, NyxAgentExecutionContext, NyxAgentRequest, RuntimeCredential,
};
use futures::TryStreamExt;

async fn stored(f: &Fixture, id: &str) -> ApprovalRequest {
    f.state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

async fn reviewed_success(
    f: &Fixture,
    row: &AssistantConversation,
) -> (
    ApprovalRequest,
    AssistantConversation,
    credentials::AssistantCredential,
) {
    run(f, row).await;
    let request = pending(f, None).await;
    proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "persistent"),
    )
    .await
    .unwrap();
    let resumed = event_turn(f, row).await;
    let credential = load_credential(f, &resumed).await;
    let egress = crate::handlers::assistant_nyxagent::http_egress(
        f.state.clone(),
        test_utils::test_auth_user(&f.owner),
    );
    assert!(matches!(
        proposals::resume_turn(&f.state, egress.as_ref(), &resumed, &credential)
            .await
            .unwrap(),
        Some(proposals::ResumeOutcome::Continue(_))
    ));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    (request, resumed, credential)
}

// Astra 1: a covered write has no proposal row. Its actual grant/effect evidence
// must still fence the GET's retained continuation; a new human root is distinct.
#[tokio::test]
async fn owner_permission_resume_private_covered_write_only_reviews_readback() {
    for version in [1, 3] {
        let (f, row) = assistant("owner_resume_covered_private").await;
        if version == 1 {
            f.state
                .db
                .collection::<Document>("security_replicas")
                .update_many(
                    doc! {},
                    doc! {"$pull":{"capabilities":proposals::v3::READINESS}},
                )
                .await
                .unwrap();
        }
        set_task(&f, &row).await;
        let cells = sheet(&f, 200).await;
        let model = model(&f, true).await;
        seed_for_chat(
            &f.state,
            &f.auth().await,
            &f.connection,
            &f.catalog,
            "PUT",
            RANGE,
        )
        .await;
        run(&f, &row).await;
        let read = pending(&f, None).await;
        assert_eq!(
            proposals::v3::AssistantVersion::of(&read).unwrap().number(),
            version
        );
        assert_eq!(
            proposals::proposal(&read).unwrap().review.operation,
            format!("GET {RANGE}")
        );
        assert_eq!(count(&f, "approval_requests").await, 1);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        approve(&f, &read, "persistent").await;
        final_reply(&f, &row, 2, &model).await;
        assert_eq!(*cells.lock().unwrap(), Some(values()));
        assert_eq!(*model.reads.lock().unwrap(), [values()]);
        assert_eq!(count(&f, "approval_requests").await, 1);
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .map(|r| r.method.as_str())
                .collect::<Vec<_>>(),
            ["PUT", "GET"]
        );
        // A genuine new human admission can spend reusable authority once.
        *cells.lock().unwrap() =
            Some(json!({"range":"Acceptance!A1:B1","values":[["old","sentinel"]]}));
        let request = engine::TurnRequest {
            conversation_id: Some(row.id.clone()),
            text: TASK.into(),
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            agent_id: None,
            model: None,
            access_mode: None,
        };
        let mut start: engine::TurnStart = (&request).into();
        start.execution_enrollment = true;
        let fresh = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
            .await
            .unwrap();
        assert_ne!(
            fresh
                .active_turn
                .as_ref()
                .unwrap()
                .execution_task
                .as_ref()
                .unwrap()
                .task,
            row.active_turn
                .as_ref()
                .unwrap()
                .execution_task
                .as_ref()
                .unwrap()
                .task
        );
        run(&f, &fresh).await;
        final_reply(&f, &fresh, 3, &model).await;
        assert_eq!(*model.reads.lock().unwrap(), [values(), values()]);
        assert_eq!(*cells.lock().unwrap(), Some(values()));
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .map(|r| r.method.as_str())
                .collect::<Vec<_>>(),
            ["PUT", "GET", "PUT", "GET"]
        );
        assert_eq!(count(&f, "approval_requests").await, 1);
        drop(model);
        f.cleanup().await;
    }
}

// Astra 1 / PM (c): two fresh operation IDs cannot race past the digest barrier.
#[tokio::test]
async fn owner_permission_resume_concurrent_digest_claim_and_historical_evidence() {
    for evidence in ["complete", "historical", "foreign", "at_limit"] {
        let historical = evidence == "historical";
        let (f, row) = assistant("owner_resume_atomic_effect").await;
        set_task(&f, &row).await;
        sheet(&f, 200).await;
        let model = model(&f, false).await;
        let (_, resumed, credential) = reviewed_success(&f, &row).await;
        assert_eq!(
            execute_with_credential(&f, &resumed, credential)
                .await
                .unwrap()
                .text,
            "Remaining task continued."
        );
        // Permit different inputs under actual reusable authority. There is no
        // earlier proposal for these inputs, so only the final digest CAS helps.
        seed_for_chat(
            &f.state,
            &f.auth().await,
            &f.connection,
            &f.catalog,
            "PUT",
            RANGE,
        )
        .await;
        let root = &resumed
            .active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .task
            .root_id;
        if historical {
            f.state.db.collection::<Document>("assistant_messages").update_one(doc! {"_id":root},
                doc! {"$unset":{"execution_dispatch_evidence_version":"","execution_dispatches":""}})
                .await.unwrap();
        }
        if evidence == "foreign" {
            let request = f
                .state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"user_id":&f.owner})
                .await
                .unwrap()
                .unwrap();
            let version = proposals::v3::AssistantVersion::of(&request).unwrap();
            f.state
                .db
                .collection::<Document>("approval_requests")
                .update_one(
                    doc! {"_id":&request.id},
                    version.update(
                        doc! {"$set":{"execution_proposal.v1.key_id":Uuid::new_v4().to_string()}},
                    ),
                )
                .await
                .unwrap();
        }
        if evidence == "at_limit" {
            let roots = f.state.db.collection::<Document>("assistant_messages");
            let saved = roots.find_one(doc! {"_id":root}).await.unwrap().unwrap();
            let entry = saved.get_array("execution_dispatches").unwrap()[0]
                .as_document()
                .unwrap();
            let entries: Vec<_> = (0..128)
                .map(|_| {
                    let mut entry = entry.clone();
                    entry.insert("operation", Uuid::new_v4().to_string());
                    entry
                })
                .collect();
            roots
                .update_one(
                    doc! {"_id":root},
                    doc! {"$set":{"execution_dispatches":entries}},
                )
                .await
                .unwrap();
        }
        let changed = json!({"range":"Acceptance!A1:B1","values":[["distinct","inputs"]]});
        let calls = async {
            tokio::join!(
                sheet_call(&f, "PUT", "Acceptance!A1:B1", Some(&changed)),
                sheet_call(&f, "PUT", "Acceptance!A1:B1", Some(&changed))
            )
        };
        let (a, b) = if historical {
            calls.await
        } else {
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                proposals::effects::BEFORE_SEND
                    .scope(Arc::new(tokio::sync::Barrier::new(2)), calls),
            )
            .await
            .expect("both independent operations must reach the actual send transaction")
        };
        assert_eq!(
            [a.0, b.0].iter().filter(|s| **s == StatusCode::OK).count(),
            usize::from(evidence == "complete")
        );
        assert!([a.0, b.0].iter().any(|s| !s.is_success()));
        assert_eq!(
            f.server.received_requests().await.unwrap().len(),
            if evidence == "complete" { 2 } else { 1 }
        );
        assert_eq!(count(&f, "approval_requests").await, 1);
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        drop(model);
        f.cleanup().await;
    }
}

struct RejectModelOnce {
    inner: Arc<dyn NyxAgentEgress>,
    status: StatusCode,
    calls: Mutex<Vec<Value>>,
}
#[async_trait::async_trait]
impl NyxAgentEgress for RejectModelOnce {
    async fn resume_execution(
        &self,
        row: &AssistantConversation,
        resume: proposals::Resume,
    ) -> AppResult<axum::response::Response> {
        self.inner.resume_execution(row, resume).await
    }
    async fn request(
        &self,
        authority: &NyxAgentExecutionContext,
        credential: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        if request.method == axum::http::Method::POST
            && request.relative_path.ends_with("/responses")
        {
            let first = {
                let mut calls = self.calls.lock().unwrap();
                calls.push(request.body.clone().unwrap());
                calls.len() == 1
            };
            if first {
                return Ok((
                    self.status,
                    Json(json!({"error":{"code":"agent_key_required",
                    "message":"Agent credential rejected"}})),
                )
                    .into_response());
            }
        }
        self.inner.request(authority, credential, request).await
    }
}

async fn execute_using_egress(
    f: &Fixture,
    row: &AssistantConversation,
    credential: &mut credentials::AssistantCredential,
    egress: &dyn NyxAgentEgress,
) -> Result<engine::TurnResult, engine::TurnError> {
    let (sender, _) = tokio::sync::broadcast::channel(256);
    Box::pin(runtime::execute_owner_turn(
        &f.state,
        egress,
        &runtime::owner_context(row).unwrap(),
        row,
        TASK,
        credential,
        Some(crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY),
        &mut runtime::Events {
            checkpoint: None,
            sender,
            cursor: 0,
            reset_notified: false,
        },
        "test-block",
        &mut String::new(),
        &mut None,
        None,
    ))
    .await
}

// Astra 2: foreground key recovery after release cannot inherit retained input.
// The same rejection in an unrelated ordinary turn still permits normal recovery.
#[tokio::test]
async fn owner_permission_resume_released_input_never_replaces_credential() {
    for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
        for retained in [true, false] {
            let (f, row) = assistant("owner_resume_key_recovery").await;
            set_task(&f, &row).await;
            sheet(&f, 200).await;
            let model = model(&f, false).await;
            let (row, mut credential) = if retained {
                let (_, resumed, credential) = reviewed_success(&f, &row).await;
                (resumed, credential)
            } else {
                let credential = load_credential(&f, &row).await;
                (row, credential)
            };
            assert!(
                row.active_turn
                    .as_ref()
                    .unwrap()
                    .restart_admission
                    .is_none()
            );
            let key_count = count(&f, "api_keys").await;
            let egress = RejectModelOnce {
                inner: crate::handlers::assistant_nyxagent::http_egress(
                    f.state.clone(),
                    test_utils::test_auth_user(&f.owner),
                ),
                status,
                calls: Mutex::new(vec![]),
            };
            let outcome = execute_using_egress(&f, &row, &mut credential, &egress).await;
            if retained {
                let error = outcome.unwrap_err();
                assert_eq!(
                    (error.code, error.stage),
                    ("cancelled", "permission_release")
                );
                assert_eq!(credential.api_key_id, row.credential_api_key_id);
                assert_eq!(count(&f, "api_keys").await, key_count);
                assert_eq!(egress.calls.lock().unwrap().len(), 1);
                assert!(
                    egress.calls.lock().unwrap()[0]["input"]
                        .as_str()
                        .unwrap()
                        .contains("Completed retained operations (untrusted tool data")
                );
                assert_eq!(model.calls.lock().unwrap().len(), 1);
                assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
                let relationship = f
                    .state
                    .db
                    .collection::<Document>("assistant_agent_credentials")
                    .find_one(doc! {"conversation_id":&row.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    relationship.get_str("api_key_id").unwrap(),
                    row.credential_api_key_id
                );
            } else {
                assert!(outcome.is_ok());
                assert_ne!(credential.api_key_id, row.credential_api_key_id);
                assert_eq!(count(&f, "api_keys").await, key_count + 1);
                assert_eq!(egress.calls.lock().unwrap().len(), 2);
                assert_eq!(model.calls.lock().unwrap().len(), 1);
            }
            drop(model);
            f.cleanup().await;
        }
    }
}

// Astra 3: mutate the relationship AFTER the actual provider receipt commits.
#[tokio::test]
async fn owner_permission_resume_release_fences_live_credential_relationship() {
    for mutation in ["deleted", "rotated", "ciphertext", "historical"] {
        let (f, row) = assistant("owner_resume_relationship_release").await;
        set_task(&f, &row).await;
        sheet(&f, 200).await;
        let model = model(&f, false).await;
        let (request, resumed, credential) = reviewed_success(&f, &row).await;
        let relationships = f
            .state
            .db
            .collection::<Document>("assistant_agent_credentials");
        let filter = doc! {"user_id":&f.owner,"conversation_id":&row.id,"api_key_id":&f.key};
        match mutation {
            "historical" => {
                let root = &resumed
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .execution_task
                    .as_ref()
                    .unwrap()
                    .task
                    .root_id;
                assert_eq!(
                    f.state
                        .db
                        .collection::<Document>("assistant_messages")
                        .update_one(
                            doc! {"_id":root},
                            doc! {"$unset":{"execution_dispatch_evidence_version":""}}
                        )
                        .await
                        .unwrap()
                        .matched_count,
                    1
                );
            }
            "deleted" => {
                assert_eq!(
                    relationships
                        .delete_one(filter)
                        .await
                        .unwrap()
                        .deleted_count,
                    1
                );
            }
            "rotated" => {
                assert_eq!(
                    relationships
                        .update_one(
                            filter,
                            doc! {"$set":{"api_key_id":Uuid::new_v4().to_string()}}
                        )
                        .await
                        .unwrap()
                        .matched_count,
                    1
                );
            }
            _ => {
                assert_eq!(relationships.update_one(filter, doc! {"$set":{"key_ciphertext":
                bson::Binary { subtype:bson::spec::BinarySubtype::Generic, bytes:vec![] }}})
                .await.unwrap().matched_count, 1);
            }
        }
        assert!(
            f.state
                .db
                .collection::<ApiKey>("api_keys")
                .find_one(doc! {"_id":&f.key})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        let outcome = execute_with_credential(&f, &resumed, credential).await;
        let request = stored(&f, &request.id).await;
        let marker = proposals::proposal(&request)
            .unwrap()
            .owner_continuation
            .as_ref()
            .unwrap();
        if mutation == "ciphertext" {
            assert_eq!(outcome.unwrap().text, "Remaining task continued.");
            assert_eq!(
                marker.released_turn_id.as_deref(),
                Some(resumed.active_turn.as_ref().unwrap().turn_id.as_str())
            );
        } else {
            let error = outcome.unwrap_err();
            assert_eq!(
                (error.code, error.stage),
                ("cancelled", "permission_release")
            );
            assert!(marker.released_turn_id.is_none());
            assert!(!marker.blocked);
        }
        assert_eq!(
            model.calls.lock().unwrap().len(),
            if mutation == "ciphertext" { 2 } else { 1 }
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        f.cleanup().await;
    }
}

struct WaitForIndependentReview {
    inner: Arc<dyn NyxAgentEgress>,
    request: String,
    independent: String,
}
#[async_trait::async_trait]
impl NyxAgentEgress for WaitForIndependentReview {
    async fn resume_execution(
        &self,
        row: &AssistantConversation,
        resume: proposals::Resume,
    ) -> AppResult<axum::response::Response> {
        if resume.request_id == self.request {
            return Err(AppError::ApprovalRequired {
                request_id: self.independent.clone(),
            });
        }
        self.inner.resume_execution(row, resume).await
    }
    async fn request(
        &self,
        authority: &NyxAgentExecutionContext,
        credential: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        self.inner.request(authority, credential, request).await
    }
}

// Astra 4 / PM (b): crash at the first success commit, before withhold. Batch IDs
// remain evidence only, including a queued sibling that could otherwise dispatch.
#[tokio::test]
async fn owner_permission_resume_partial_batch_crash_reconciles_success_without_expanding_wake() {
    for sibling in [
        "denied",
        "waiting",
        "queued",
        "turn_drift",
        "task_drift",
        "credential_drift",
    ] {
        let (f, row) = assistant("owner_resume_batch_crash").await;
        set_task(&f, &row).await;
        sheet(&f, 200).await;
        let model = model(&f, sibling == "queued").await;
        assert_eq!(
            call(f.router(), &f.raw, "PUT").await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            sheet_call(&f, "PUT", "Other!A1:B1", Some(&values()))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        run(&f, &row).await;
        let requests: Vec<_> = f
            .state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find(doc! {"status":"pending"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(requests.len(), 2);
        let success = requests
            .iter()
            .find(|r| {
                !proposals::proposal(r)
                    .unwrap()
                    .review
                    .operation
                    .contains("Other!")
            })
            .unwrap();
        let other = requests.iter().find(|r| r.id != success.id).unwrap();
        for request in [&success, &other] {
            let mut input = decision(request, "persistent");
            if request.id == other.id && sibling == "denied" {
                input.approved = false;
                input.choice_id = None;
                input.choice_digest = None;
            }
            proposals::decide(&f.state, &f.owner, &request.id, &input)
                .await
                .unwrap();
        }
        let mut resumed = event_turn(&f, &row).await;
        let independent = crate::services::approval_service::create_tool_approval_request(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            None,
            None,
            &f.owner,
            "independent-review-fixture",
            None,
            None,
            false,
            "agent",
            &f.key,
            None,
            Default::default(),
        )
        .await
        .unwrap();
        let egress = WaitForIndependentReview {
            inner: crate::handlers::assistant_nyxagent::http_egress(
                f.state.clone(),
                test_utils::test_auth_user(&f.owner),
            ),
            request: if sibling == "waiting" {
                other.id.clone()
            } else {
                String::new()
            },
            independent: independent.id,
        };
        // Prime the waiting branch first; the other branches crash before even
        // considering their sibling. All IDs are real admitted decision events.
        resumed
            .active_turn
            .as_mut()
            .unwrap()
            .events
            .sort_by_key(|e| {
                (e.id == format!("execution-decision:{}", success.id)) == (sibling == "waiting")
            });
        let mut credential = load_credential(&f, &resumed).await;
        let interrupted = proposals::continuation::FAIL_AFTER_RECEIPT
            .scope(
                true,
                execute_using_egress(&f, &resumed, &mut credential, &egress),
            )
            .await;
        if interrupted.is_ok() {
            let saved = stored(&f, &success.id).await;
            let p = proposals::proposal(&saved).unwrap();
            let proof = p.owner_continuation.as_ref().map(|proof| {
                (
                    proof.version,
                    proof.blocked,
                    proof.batch.as_ref().map(Vec::len),
                    proof.batch_turn_id.is_some(),
                )
            });
            panic!(
                "expected post-success receipt interruption: sibling={sibling}, status={}, state={}, activity={:?}, proof={proof:?}, provider_attempts={}, model_calls={}",
                saved.status,
                p.state,
                saved.activity_outcome,
                f.server.received_requests().await.unwrap().len(),
                model.calls.lock().unwrap().len()
            );
        }
        let error = interrupted.unwrap_err();
        assert_eq!(error.stage, "permission_resume");
        let committed = stored(&f, &success.id).await;
        let marker = proposals::proposal(&committed)
            .unwrap()
            .owner_continuation
            .as_ref()
            .unwrap();
        assert_eq!(marker.version, 3);
        assert_eq!(marker.batch.as_ref().unwrap().len(), 2);
        assert!(!marker.blocked);
        assert!(marker.released_turn_id.is_none());
        if sibling != "denied" {
            let partner = stored(&f, &other.id).await;
            let partner_marker = proposals::proposal(&partner)
                .unwrap()
                .owner_continuation
                .as_ref()
                .unwrap();
            assert_eq!(partner_marker.batch, marker.batch);
            assert_eq!(partner_marker.version, 3);
        }
        if sibling == "waiting" {
            assert_eq!(
                proposals::proposal(&stored(&f, &other.id).await)
                    .unwrap()
                    .state,
                "waiting"
            );
        }
        engine::finish_turn(
            &f.state.db,
            &resumed,
            &f.key,
            &Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: String::new(),
                session_id: None,
                response_id: None,
                error: Some(error),
            },
        )
        .await
        .unwrap();
        for request in [&success, &other] {
            let version = proposals::v3::AssistantVersion::of(request).unwrap();
            f.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&request.id},
                version.update(doc! {"$set":{"execution_proposal.v1.wake_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1)),
                    "execution_proposal.v1.maintenance_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1))}}))
                .await.unwrap();
        }
        if matches!(sibling, "turn_drift" | "task_drift" | "credential_drift") {
            let version = proposals::v3::AssistantVersion::of(other).unwrap();
            let changed = match sibling {
                "turn_drift" => {
                    doc! {"execution_proposal.v1.owner_continuation.batch_turn_id":Uuid::new_v4().to_string()}
                }
                "task_drift" => doc! {"execution_proposal.v1.task.epoch":2_i64},
                _ => doc! {"execution_proposal.v1.key_id":Uuid::new_v4().to_string()},
            };
            f.state
                .db
                .collection::<Document>("approval_requests")
                .update_one(
                    doc! {"_id":&other.id},
                    version.update(doc! {"$set":changed}),
                )
                .await
                .unwrap();
        }
        proposals::sweep(&f.state).await.unwrap();
        // Model a lost sibling queue entry after maintenance. Its stored batch
        // membership cannot authorize its dispatch or a success-only model run.
        f.state
            .db
            .collection::<Document>("assistant_conversations")
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$pull":{"pending_events":{"id":format!("execution-decision:{}", other.id)}}},
            )
            .await
            .unwrap();
        let recovered = event_turn(&f, &row).await;
        assert_eq!(recovered.active_turn.as_ref().unwrap().events.len(), 1);
        run(&f, &recovered).await;
        let messages = engine::messages(&f.state.db, &f.owner, &row.id, 20, None)
            .await
            .unwrap();
        assert_eq!(
            messages.last().unwrap().text,
            proposals::result_for_human(&f.state, &committed)
                .await
                .unwrap()
                .unwrap()
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert_eq!(model.calls.lock().unwrap().len(), 1);
        assert_eq!(
            proposals::proposal(&stored(&f, &success.id).await)
                .unwrap()
                .owner_continuation
                .as_ref()
                .unwrap()
                .blocked,
            sibling != "queued"
        );
        if sibling == "queued" {
            wiremock::Mock::given(wiremock::matchers::method("PUT"))
                .and(wiremock::matchers::path(
                    "/v4/spreadsheets/book/values/Other!A1:B1",
                ))
                .respond_with(
                    wiremock::ResponseTemplate::new(200).set_body_json(json!({"updatedCells":2})),
                )
                .mount(&f.server)
                .await;
            let version = proposals::v3::AssistantVersion::of(other).unwrap();
            f.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&other.id},
                version.update(doc! {"$set":{"execution_proposal.v1.wake_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1)),
                    "execution_proposal.v1.maintenance_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1))}}))
                .await.unwrap();
            proposals::sweep(&f.state).await.unwrap();
            let partner_turn = event_turn(&f, &row).await;
            assert_eq!(partner_turn.active_turn.as_ref().unwrap().events.len(), 1);
            seed_for_chat(
                &f.state,
                &f.auth().await,
                &f.connection,
                &f.catalog,
                "GET",
                RANGE,
            )
            .await;
            // Lose the model handoff after B's genuine provider success too.
            // Both receipts must remain recoverable, with no new model call.
            let error = proposals::continuation::FAIL_AFTER_RECEIPT
                .scope(true, execute(&f, &partner_turn))
                .await
                .unwrap_err();
            assert_eq!(error.stage, "permission_resume");
            assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
            assert_eq!(model.calls.lock().unwrap().len(), 1);
            engine::finish_turn(
                &f.state.db,
                &partner_turn,
                &f.key,
                &Uuid::new_v4().to_string(),
                &engine::TurnResult {
                    text: String::new(),
                    session_id: None,
                    response_id: None,
                    error: Some(error),
                },
            )
            .await
            .unwrap();
            for request in [success, other] {
                let version = proposals::v3::AssistantVersion::of(request).unwrap();
                f.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&request.id},
                    version.update(doc! {"$set":{"execution_proposal.v1.wake_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1)),
                        "execution_proposal.v1.maintenance_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1))}}))
                    .await.unwrap();
            }
            proposals::sweep(&f.state).await.unwrap();
            let pending = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert_eq!(pending.pending_events.len(), 2);
            // Only A is admitted now. B's success is receipt evidence, never
            // another executable effect or a second final channel delivery.
            f.state.db.collection::<Document>("assistant_conversations").update_one(doc! {"_id":&row.id},
                doc! {"$pull":{"pending_events":{"id":format!("execution-decision:{}", other.id)}}})
                .await.unwrap();
            let partner_turn = event_turn(&f, &row).await;
            assert_eq!(partner_turn.active_turn.as_ref().unwrap().events.len(), 1);
            run(&f, &partner_turn).await;
            final_reply(&f, &partner_turn, 2, &model).await;
            assert_eq!(*model.reads.lock().unwrap(), [values()]);
            let messages = engine::messages(&f.state.db, &f.owner, &row.id, 20, None)
                .await
                .unwrap();
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message.role == "assistant" && message.text == ANSWER)
                    .count(),
                1
            );
            assert_eq!(messages.last().unwrap().conversation_id, row.id);
            assert_eq!(messages.last().unwrap().text, answer(&values()));
            let partner = stored(&f, &other.id).await;
            let partner_marker = proposals::proposal(&partner)
                .unwrap()
                .owner_continuation
                .as_ref()
                .unwrap();
            assert_eq!(partner_marker.batch, marker.batch);
            assert!(!partner_marker.blocked);
            assert_eq!(partner_marker.batch_turn_id, marker.batch_turn_id);
            let final_turn = &partner_turn.active_turn.as_ref().unwrap().turn_id;
            for id in [&success.id, &other.id] {
                let receipt = stored(&f, id).await;
                let receipt = proposals::proposal(&receipt).unwrap();
                assert_eq!(
                    receipt.delivered_turn_id.as_deref(),
                    Some(final_turn.as_str())
                );
                assert_eq!(
                    receipt
                        .owner_continuation
                        .as_ref()
                        .unwrap()
                        .released_turn_id
                        .as_deref(),
                    Some(final_turn.as_str())
                );
            }
            // This was its own real approved decision; it was not dispatched
            // by expanding the earlier success-only wake's stored IDs.
            assert_eq!(
                f.server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .map(|request| request.method.as_str())
                    .collect::<Vec<_>>(),
                ["PUT", "PUT", "GET"]
            );
            assert_eq!(model.calls.lock().unwrap().len(), 2);
            proposals::sweep(&f.state).await.unwrap();
            assert!(
                engine::get(&f.state.db, &f.owner, &row.id)
                    .await
                    .unwrap()
                    .pending_events
                    .is_empty()
            );
            assert!(
                execute(&f, &partner_turn)
                    .await
                    .unwrap()
                    .text
                    .contains("HTTP 200")
            );
            assert_eq!(model.calls.lock().unwrap().len(), 2);
            assert_eq!(f.server.received_requests().await.unwrap().len(), 3);
        }
        drop(model);
        f.cleanup().await;
    }
}

// Astra 5: an HTTP 202 body is readable acceptance, never provider completion.
#[tokio::test]
async fn owner_permission_resume_http_202_is_pending_without_readback_or_resend() {
    const NOTICE: &str = "The provider accepted the operation (HTTP 202); completion is pending and unverified. It will not be sent again.";
    let (f, row) = assistant("owner_resume_http_accepted").await;
    set_task(&f, &row).await;
    let cells = sheet(&f, 202).await;
    let model = model(&f, true).await;
    run(&f, &row).await;
    let request = pending(&f, None).await;
    proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "persistent"),
    )
    .await
    .unwrap();
    let resumed = event_turn(&f, &row).await;
    assert_eq!(execute(&f, &resumed).await.unwrap().text, NOTICE);
    let committed = stored(&f, &request.id).await;
    let receipt = proposals::proposal(&committed).unwrap();
    assert_eq!(committed.activity_outcome.as_deref(), Some("pending"));
    assert_eq!(receipt.state, "unknown");
    assert_eq!(
        receipt.failure_code.as_deref(),
        Some("provider_completion_pending")
    );
    assert!(receipt.owner_continuation.as_ref().unwrap().blocked);
    assert_eq!(
        proposals::result_for_human(&f.state, &committed)
            .await
            .unwrap()
            .as_deref(),
        Some(NOTICE)
    );
    assert_eq!(execute(&f, &resumed).await.unwrap().text, NOTICE);
    engine::finish_turn(
        &f.state.db,
        &resumed,
        &f.key,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(engine::TurnError::internal(
                "assistant_unavailable",
                "permission_resume",
            )),
        },
    )
    .await
    .unwrap();
    let version = proposals::v3::AssistantVersion::of(&request).unwrap();
    // The receipt was never delivered. Maintenance recovers that exact wake.
    f.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&request.id}, version.update(doc! {
        "$set":{"execution_proposal.v1.wake_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1)),
            "execution_proposal.v1.maintenance_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1))}}))
        .await.unwrap();
    proposals::sweep(&f.state).await.unwrap();
    let recovered = event_turn(&f, &row).await;
    assert_eq!(execute(&f, &recovered).await.unwrap().text, NOTICE);
    assert_eq!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.method.as_str())
            .collect::<Vec<_>>(),
        ["PUT"]
    );
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    assert!(model.reads.lock().unwrap().is_empty());
    assert!(cells.lock().unwrap().is_none());
    assert_eq!(count(&f, "approval_requests").await, 1);
    f.cleanup().await;
}

// A successful decision co-admitted with an ordinary event never acquires a
// release marker, even if its ordinary partner is absent from later recovery.
#[tokio::test]
async fn owner_permission_resume_mixed_ordinary_event_never_becomes_success_only() {
    let (f, row) = assistant("owner_resume_mixed_ordinary").await;
    set_task(&f, &row).await;
    sheet(&f, 200).await;
    let model = model(&f, false).await;
    run(&f, &row).await;
    let request = pending(&f, None).await;
    proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "persistent"),
    )
    .await
    .unwrap();
    let mut follow = crate::services::assistant_team_service::event(
        "followup",
        "Continue the admitted task".into(),
        None,
    );
    follow.execution_task = row.active_turn.as_ref().unwrap().execution_task.clone();
    engine::push_events(&f.state.db, &f.owner, &row.id, vec![follow])
        .await
        .unwrap()
        .unwrap();
    let mut resumed = event_turn(&f, &row).await;
    assert_eq!(resumed.active_turn.as_ref().unwrap().events.len(), 2);
    assert!(
        execute(&f, &resumed)
            .await
            .unwrap()
            .text
            .contains("HTTP 200")
    );
    assert!(
        proposals::proposal(&stored(&f, &request.id).await)
            .unwrap()
            .owner_continuation
            .as_ref()
            .is_none_or(|proof| proof.blocked)
    );
    resumed
        .active_turn
        .as_mut()
        .unwrap()
        .events
        .retain(|e| e.id.starts_with("execution-decision:"));
    assert!(
        execute(&f, &resumed)
            .await
            .unwrap()
            .text
            .contains("HTTP 200")
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    drop(model);
    f.cleanup().await;
}

// Round two: ordinary historical/readiness-off coverage has no v2 evidence cap.
// Exercise the actual 129th provider request, plus the marked saturation control.
#[tokio::test]
async fn owner_permission_resume_evidence_capacity_only_applies_to_marked_roots() {
    for kind in ["current", "legacy", "readiness_off"] {
        let (f, mut row) = assistant("owner_evidence_capacity_scope").await;
        if kind == "readiness_off" {
            finish(&f, &row, "Fixture setup complete.").await;
            // The default fixture heartbeat replaces its original capability
            // list. Stop it before modeling a stable older serving replica.
            crate::services::security_readiness_service::stop_test_replica(&f.state.db).await;
            f.state
                .db
                .collection::<Document>("security_replicas")
                .update_many(
                    doc! {},
                    doc! {"$pull":{"capabilities":proposals::continuation::READINESS}},
                )
                .await
                .unwrap();
            let request = engine::TurnRequest {
                conversation_id: Some(row.id.clone()),
                text: TASK.into(),
                acknowledgement_id: None,
                retry_of: None,
                attachment_ids: vec![],
                agent_id: None,
                model: None,
                access_mode: None,
            };
            let mut start: engine::TurnStart = (&request).into();
            start.execution_enrollment = true;
            row = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
                .await
                .unwrap();
        }
        let task = row
            .active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap();
        let roots = f.state.db.collection::<Document>("assistant_messages");
        if kind == "legacy" {
            roots
                .update_one(
                    doc! {"_id":&task.task.root_id},
                    doc! {"$unset":{"execution_dispatch_evidence_version":""}},
                )
                .await
                .unwrap();
        }
        let grant = seed_for_chat(
            &f.state,
            &f.auth().await,
            &f.connection,
            &f.catalog,
            "PUT",
            RANGE,
        )
        .await;
        // Historical metadata at the old bound cannot enroll an unmarked root.
        let old_evidence: Vec<_> = (0..128)
            .map(|_| {
                doc! {
                    "version":1,"epoch":task.epoch,"key":&f.key,"conversation":&row.id,
                    "connection":&f.connection,"digest":"historical-fixture-digest",
                    "operation":Uuid::new_v4().to_string(),"grant":&grant.id,
                    "turn":&row.active_turn.as_ref().unwrap().turn_id,
                }
            })
            .collect();
        if kind != "current" {
            roots
                .update_one(
                    doc! {"_id":&task.task.root_id},
                    doc! {"$set":{"execution_dispatches":&old_evidence}},
                )
                .await
                .unwrap();
            let saved = roots
                .find_one(doc! {"_id":&task.task.root_id})
                .await
                .unwrap()
                .unwrap();
            assert!(!saved.contains_key("execution_dispatch_evidence_version"));
        }
        let cells = sheet(&f, 200).await;
        for number in 1..=128 {
            if kind == "readiness_off" {
                refresh_test_replica_heartbeat(&f.state.db).await;
            }
            assert_eq!(
                call(f.router(), &f.raw, "PUT").await.0,
                StatusCode::OK,
                "{kind} ordinary covered write {number}"
            );
        }
        assert_eq!(f.server.received_requests().await.unwrap().len(), 128);
        let before = roots
            .find_one(doc! {"_id":&task.task.root_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.get_array("execution_dispatches").unwrap().len(), 128);
        if kind == "readiness_off" {
            refresh_test_replica_heartbeat(&f.state.db).await;
        }
        let (status, _) = call(f.router(), &f.raw, "PUT").await;
        assert_eq!(
            status,
            if kind == "current" {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::OK
            }
        );
        assert_eq!(
            f.server.received_requests().await.unwrap().len(),
            if kind == "current" { 128 } else { 129 }
        );
        let after = roots
            .find_one(doc! {"_id":&task.task.root_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.get("execution_dispatches"),
            before.get("execution_dispatches")
        );
        assert_eq!(
            after.get("execution_dispatch_evidence_version"),
            before.get("execution_dispatch_evidence_version")
        );
        assert_eq!(*cells.lock().unwrap(), Some(values()));
        assert_eq!(count(&f, "approval_requests").await, 0);
        assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
        f.cleanup().await;
    }
}

#[tokio::test]
async fn owner_permission_resume_optional_evidence_readiness_failure_keeps_ordinary_admission() {
    let (f, row) = assistant("owner_evidence_optional_readiness").await;
    finish(&f, &row, "Fixture setup complete.").await;
    readiness_dependency(&f, true).await;
    let request = engine::TurnRequest {
        conversation_id: Some(row.id.clone()),
        text: TASK.into(),
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&request).into();
    start.execution_enrollment = true;
    let fresh = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    let task = fresh
        .active_turn
        .as_ref()
        .unwrap()
        .execution_task
        .as_ref()
        .unwrap();
    let root = f
        .state
        .db
        .collection::<Document>("assistant_messages")
        .find_one(doc! {"_id":&task.task.root_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!root.contains_key("execution_dispatch_evidence_version"));
    assert!(!root.contains_key("execution_dispatches"));
    seed_for_chat(
        &f.state,
        &f.auth().await,
        &f.connection,
        &f.catalog,
        "PUT",
        RANGE,
    )
    .await;
    sheet(&f, 200).await;
    assert_eq!(call(f.router(), &f.raw, "PUT").await.0, StatusCode::OK);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    let root = f
        .state
        .db
        .collection::<Document>("assistant_messages")
        .find_one(doc! {"_id":&task.task.root_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!root.contains_key("execution_dispatches"));
    f.cleanup().await;
}

#[path = "proposal_owner_continuation_recovery_tests.rs"]
mod recovery_cases;
