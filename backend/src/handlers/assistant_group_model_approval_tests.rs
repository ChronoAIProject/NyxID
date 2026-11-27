//! Real turn -> local NyxAgent stand-in -> authenticated proxy -> local model.
use super::*;
use crate::handlers::credential_record_fixtures::set_catalog_credential;
use crate::models::{approval_request::ApprovalRequest, audit_log::AuditLog};
use crate::services::execution_permission_dispatch::proposals::{self, ModelApprovalHook};
use axum::{
    Extension,
    body::Body,
    http::{HeaderMap, Request},
    routing::any,
};
use mongodb::bson;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProposalCase {
    New,
    NextTurn,
    Existing,
    ExistingDecided,
    Limit,
    TaskFence,
    BindAbort,
}

impl ProposalCase {
    fn hook(self) -> ModelApprovalHook {
        let prepared = Arc::new(std::sync::atomic::AtomicBool::new(false));
        Arc::new(move |db, request| {
            let prepared = prepared.clone();
            Box::pin(async move {
                if prepared.swap(true, Ordering::SeqCst) {
                    return;
                }
                match self {
                    Self::New | Self::NextTurn | Self::BindAbort => {}
                    Self::Existing | Self::ExistingDecided => {
                        let mut request = request;
                        if self == Self::ExistingDecided {
                            request.status = "denied".into();
                            request.decided_at = Some(Utc::now());
                            let envelope = request.execution_proposal.as_mut().unwrap();
                            envelope
                                .v1
                                .as_mut()
                                .or(envelope.v3.as_mut())
                                .or(envelope.v4.as_mut())
                                .unwrap()
                                .state = "denied".into();
                        }
                        db.collection::<ApprovalRequest>("approval_requests")
                            .insert_one(&request)
                            .await
                            .unwrap();
                        crate::services::audit_service::log_system_event(
                            db.clone(),
                            "execution_permission_requested",
                            Some(json!({"request_id":request.id})),
                        )
                        .await
                        .unwrap();
                    }
                    Self::Limit => {
                        let mut rows = Vec::new();
                        for index in 0..32 {
                            let mut row = bson::to_document(&request).unwrap();
                            row.insert("_id", format!("existing-{index}"));
                            row.insert("idempotency_key", format!("existing-{index}"));
                            let version = format!(
                                "v{}",
                                request.execution_proposal.as_ref().unwrap().version
                            );
                            row.get_document_mut("execution_proposal")
                                .unwrap()
                                .get_document_mut(&version)
                                .unwrap()
                                .insert("gap_key", format!("existing-{index}"));
                            rows.push(row);
                        }
                        db.collection::<bson::Document>("approval_requests")
                            .insert_many(rows)
                            .await
                            .unwrap();
                    }
                    Self::TaskFence => {
                        let task = &proposals::proposal(&request).unwrap().task;
                        let changed = db
                            .collection::<bson::Document>("assistant_messages")
                            .update_one(
                                doc! {"_id":&task.task.root_id},
                                doc! {"$inc":{"execution_task.epoch":1_i64}},
                            )
                            .await
                            .unwrap();
                        assert_eq!(changed.matched_count, 1);
                    }
                }
            })
        })
    }
}

#[derive(Clone, Default)]
struct AbortObservation {
    callbacks: Arc<AtomicUsize>,
    pending: Arc<std::sync::atomic::AtomicBool>,
    finalizations: Arc<AtomicUsize>,
}

async fn callback_proxy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((slug, path)): Path<(String, String)>,
    Extension(hook): Extension<ModelApprovalHook>,
    Extension((case, abort)): Extension<(ProposalCase, AbortObservation)>,
    Extension(settled): Extension<Option<tokio::sync::watch::Sender<()>>>,
    request: Request<Body>,
) -> AppResult<axum::response::Response> {
    // Keep the proposal write slower than several 250 ms turn-stop polls.
    let call = proposals::MODEL_APPROVAL_PREPARE.scope(
        hook,
        proposals::MODEL_APPROVAL_SETTLED.scope(
            settled.map(|settled| settled.subscribe()),
            proposals::MODEL_APPROVAL_DELAY.scope(Duration::from_millis(750), async {
                Box::pin(crate::handlers::proxy::proxy_request_by_slug_inner(
                    &state,
                    &auth,
                    &slug,
                    &path,
                    request,
                    &mut String::new(),
                ))
                .await
            }),
        ),
    );
    if case == ProposalCase::BindAbort {
        use crate::services::assistant_activity_decision as activity;
        abort.callbacks.fetch_add(1, Ordering::Relaxed);
        activity::DECISION_PENDING
            .scope(
                abort.pending.clone(),
                activity::PROXY_FINALIZATIONS.scope(
                    abort.finalizations.clone(),
                    proposals::MODEL_APPROVAL_ABORT_AFTER_BIND.scope(true, call),
                ),
            )
            .await
    } else {
        call.await
    }
}
async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{address}"), task)
}

async fn model_approval_pipeline(origin: &'static str, policy: &'static str, permissions: bool) {
    Box::pin(model_approval_pipeline_case(
        origin,
        policy,
        permissions,
        ProposalCase::New,
    ))
    .await;
}

async fn model_approval_pipeline_case(
    origin: &'static str,
    policy: &'static str,
    permissions: bool,
    case: ProposalCase,
) {
    Box::pin(model_approval_pipeline_order(
        origin,
        policy,
        permissions,
        case,
        false,
    ))
    .await;
}

async fn model_approval_pipeline_order(
    origin: &'static str,
    policy: &'static str,
    permissions: bool,
    case: ProposalCase,
    hold_after_commit: bool,
) {
    let (state, _, original) = setup("model_approval_pipeline").await;
    original.abort();
    let nyxbot = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    if permissions {
        crate::services::execution_permission_dispatch::phase_b_tests::enroll(&state.db, OWNER)
            .await;
    }
    let model_calls = Arc::new(AtomicUsize::new(0));
    let requests = model_calls.clone();
    let (model_url, model_task) = serve(Router::new().route("/v1/responses", route_post(
        move |headers: HeaderMap| {
            requests.fetch_add(1, Ordering::SeqCst);
            assert_eq!(headers["authorization"], "Bearer test-only-master");
            async { ([("content-type", "text/event-stream")],
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"OK\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n") }
        }
    ))).await;
    let mut model = crate::models::downstream_service::test_helpers::dummy_service();
    model.id = Uuid::new_v4().to_string();
    model.slug = "crew-approval-model".into();
    model.base_url = model_url.clone();
    model.auth_method = "bearer".into();
    model.service_category = "internal".into();
    model.requires_user_credential = false;
    set_catalog_credential(&state.encryption_keys, &mut model, b"test-only-master").await;
    model.platform_key =
        Some(serde_json::from_value(json!({"enabled":true,"audience":"public"})).unwrap());
    model.inference = Some(
        serde_json::from_value(json!({"wire_protocol":"openai_responses","model_list":true}))
            .unwrap(),
    );
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(&model)
        .await
        .unwrap();
    let connection = crate::services::assistant_authority_tests::connected(
        &state.db,
        OWNER,
        &model.slug,
        &model_url,
    )
    .await;
    state.db.collection::<mongodb::bson::Document>("user_services").update_one(doc!{"_id":&connection},
        doc!{"$set":{"catalog_service_id":&model.id,"credential_binding":"platform","auth_method":"bearer",
            "source":if matches!(policy,"all"|"write"|"write_full"|"manual_default"|"org") {"manual"} else {"auto_provision"}}}).await.unwrap();
    let org = Uuid::new_v4().to_string();
    if policy == "org" {
        use crate::models::{org_membership::OrgRole, user::UserType};
        state
            .db
            .collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(&org, UserType::Org))
            .await
            .unwrap();
        state
            .db
            .collection::<crate::models::org_membership::OrgMembership>("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &org,
                OWNER,
                OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(doc! {"_id":&connection}, doc! {"$set":{"user_id":&org}})
            .await
            .unwrap();
        let moved = state
            .db
            .collection::<bson::Document>("user_endpoints")
            .update_many(
                doc! {"user_id":OWNER,"url":&model_url},
                doc! {"$set":{"user_id":&org}},
            )
            .await
            .unwrap();
        assert_eq!(moved.matched_count, 1);
    }
    if matches!(policy, "org" | "write_full") {
        // Org policy still refuses with independent legacy connection consent
        // covered. Personal non-stateless requests retain Full-access behavior.
        crate::services::assistant_settings_service::update(
            &state.db,
            OWNER,
            crate::services::assistant_settings_service::Update {
                access_mode: Some(crate::models::assistant_settings::AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let channel = crate::services::notification_service::get_or_create_channel(&state.db, OWNER)
        .await
        .unwrap();
    state.db.collection::<mongodb::bson::Document>("notification_channels").update_one(doc!{"_id":channel.id},
        doc!{"$set":{"approval_timeout_secs":300,"approval_required":if policy=="all" {bson::Bson::Boolean(true)} else {bson::Bson::Null}}}).await.unwrap();
    if matches!(policy, "explicit" | "deny" | "org") {
        state.db.collection::<mongodb::bson::Document>("service_approval_configs").insert_one(doc!{
            "_id":Uuid::new_v4().to_string(),"user_id":if policy=="org" {org.as_str()} else {OWNER},"service_id":&model.id,"service_name":"Test model",
            "approval_required":true,"approval_mode":"per_request","rules":[],
            "default_effect":if policy=="deny" {bson::Bson::String("deny".into())} else {bson::Bson::Null},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).await.unwrap();
    }
    let (settlement_signal, _) = tokio::sync::watch::channel(());
    let abort = AbortObservation::default();
    let callbacks = Router::new()
        .route("/api/v1/proxy/s/{slug}/{*path}", any(callback_proxy))
        .layer(Extension(
            crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY,
        ))
        .layer(Extension(case.hook()))
        .layer(Extension((case, abort.clone())))
        .layer(Extension(
            hold_after_commit.then(|| settlement_signal.clone()),
        ))
        .with_state(state.clone());
    let callback_calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = callback_calls.clone();
    let callback_uri = if policy == "org" {
        format!("/api/v1/proxy/s/crew-approval-model/v1/responses?_nyxid_via={connection}")
    } else {
        "/api/v1/proxy/s/crew-approval-model/v1/responses".into()
    };
    let (agent_url, agent_task) = serve(Router::new().route("/v1/responses", route_post(
        move |headers: HeaderMap, Json(body): Json<Value>| {
            let callbacks = callbacks.clone(); let calls = observed_calls.clone(); let callback_uri = callback_uri.clone();
            async move {
                let stream = async_stream::stream! {
                    yield Ok::<_, Infallible>(Event::default().comment("keepalive"));
                    for attempt in 0..6 {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let mut input = json!({"model":"test-model","input":body["input"],"store":false,"stream":true});
                        if matches!(policy,"write"|"write_full") {input["unrecognized_codex_field"] = json!(attempt);}
                        let request = Request::builder().method("POST").uri(&callback_uri)
                            .header("authorization", headers["authorization"].clone())
                            .header("content-type", "application/json").header("x-client-attempt",attempt.to_string())
                            .body(Body::from(input.to_string())).unwrap();
                        let response = callbacks.clone().oneshot(request).await.unwrap();
                        let success = response.status().is_success();
                        let _ = axum::body::to_bytes(response.into_body(),100_000).await.unwrap();
                        if success {
                            let session = format!("conv_{}",Uuid::new_v4().simple());
                            let response = format!("resp_{}_{}",&session[5..],Uuid::new_v4().simple());
                            yield Ok(Event::default().data(json!({"type":"response.completed","sequence_number":1,
                                "response":{"id":response,"conversation":{"id":session},"status":"completed",
                                    "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"CREW TEST OK"}]}]}}).to_string()));
                            return;
                        }
                        // Model retries change request metadata, as real engine attempts can.
                        yield Ok(Event::default().comment("keepalive"));
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    yield Ok(Event::default().data(json!({"type":"response.failed","sequence_number":1,
                        "response":{"error":{"code":"turn_failed"}}}).to_string()));
                };
                Sse::new(stream).into_response()
            }
        }
    ))).await;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":agent_url}},
        )
        .await
        .unwrap();
    let interactive = origin == "user" && !permissions && policy == "write";
    let approving = if interactive {
        let state = state.clone();
        Some(tokio::spawn(async move {
            let request = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Some(request) = state
                        .db
                        .collection::<ApprovalRequest>("approval_requests")
                        .find_one(doc! {"status":"pending"})
                        .await
                        .unwrap()
                    {
                        break request;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(request.http_method.as_deref(), Some("POST"));
            assert!(
                request
                    .expires_at
                    .signed_duration_since(request.created_at)
                    .num_seconds()
                    >= 299
            );
            // Cancellation of the connected wait does not decide or expire its card.
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(120),
                    crate::services::approval_service::wait_for_decision(
                        &state.db,
                        &request.id,
                        300
                    )
                )
                .await
                .is_err()
            );
            assert_eq!(
                crate::services::approval_service::get_request(&state.db, &request.id)
                    .await
                    .unwrap()
                    .status,
                "pending"
            );
            crate::services::approval_service::process_decision(
                &state.db,
                &state.config,
                &state.http_client,
                None,
                None,
                &request.id,
                true,
                None,
                None,
                "web",
            )
            .await
            .unwrap();
        }))
    } else {
        None
    };
    let start = std::time::Instant::now();
    let (conversation, group_rows) = if origin == "crew" {
        let group = group_of(&state, "QA Crew", &[&nyxbot]).await;
        send(&state, &group.id, "Reply with only CREW TEST OK", None)
            .await
            .unwrap();
        let rows = settled(&state, &group.id, 3).await;
        (
            groups::member_thread(&state.db, OWNER, &group.id, &nyxbot.id)
                .await
                .unwrap()
                .unwrap(),
            Some(rows),
        )
    } else {
        let input: engine::TurnRequest = serde_json::from_value(
            json!({"text":"Reply with only CREW TEST OK","agent_id":nyxbot.id}),
        )
        .unwrap();
        let mut input = TurnStart::from(&input);
        input.origin = match origin {
            "event" => TurnOrigin::Event,
            "channel" => TurnOrigin::Channel,
            _ => TurnOrigin::User,
        };
        input.execution_enrollment = permissions;
        if origin == "event" {
            let home = team::home_thread(&state.db, &state.encryption_keys, &nyxbot)
                .await
                .unwrap();
            engine::push_events(
                &state.db,
                OWNER,
                &home.id,
                vec![team::bound_event(
                    "subagent_settled",
                    "Reply with only CREW TEST OK".into(),
                    None,
                    None,
                )],
            )
            .await
            .unwrap();
            input.conversation_id = Some(home.id);
        }

        let Started::Turn { conversation, .. } = start_server_turn(
            &state,
            OWNER,
            input,
            Pool::Team {
                owner: OWNER,
                limit: 2,
            },
        )
        .await
        .unwrap() else {
            panic!("not started")
        };
        tokio::time::timeout(Duration::from_secs(15), async {
            while engine::get(&state.db, OWNER, &conversation.id)
                .await
                .unwrap()
                .active_turn
                .is_some()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        (*conversation, None)
    };
    settlement_signal.send_replace(());
    if let Some(approving) = approving {
        approving.await.unwrap();
    }
    assert!(start.elapsed() < Duration::from_secs(15));
    let messages = engine::messages(&state.db, OWNER, &conversation.id, 20, None)
        .await
        .unwrap();
    let reply = messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .unwrap();
    if case == ProposalCase::BindAbort {
        assert_eq!(abort.callbacks.load(Ordering::Relaxed), 1);
        assert!(
            !abort.pending.load(Ordering::Relaxed),
            "aborted proposal marked decision-required"
        );
        assert_eq!(
            abort.finalizations.load(Ordering::Relaxed),
            0,
            "aborted proposal finalized an unstored activity"
        );
        assert!(
            reply.activities.is_empty(),
            "aborted activity survived settlement"
        );
    }
    let success = interactive || !matches!(policy, "deny" | "write" | "org");
    let approvals = state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .count_documents(doc! {})
        .await
        .unwrap();
    assert!(approvals <= if case == ProposalCase::Limit { 32 } else { 1 });
    if case != ProposalCase::New {
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role == "assistant")
                .count(),
            1
        );
    }
    assert_eq!(
        callback_calls.load(Ordering::SeqCst),
        1,
        "{origin}/{policy}: {:?}",
        reply.error_code
    );
    if success {
        assert!(reply.error_code.is_none(), "{:?}", reply.error_code);
        assert_eq!(reply.text, "CREW TEST OK");
        assert_eq!(model_calls.load(Ordering::SeqCst), 1);
        assert_eq!(approvals, u64::from(interactive));
    } else {
        assert_eq!(
            approvals,
            match case {
                ProposalCase::Limit => 32,
                ProposalCase::TaskFence | ProposalCase::BindAbort => 0,
                _ => u64::from(permissions && origin == "user" && policy == "write"),
            }
        );
        assert_eq!(
            state
                .db
                .collection::<AuditLog>("audit_log")
                .count_documents(doc! {"event_type":"execution_permission_requested"})
                .await
                .unwrap(),
            u64::from(
                permissions
                    && origin == "user"
                    && policy == "write"
                    && !matches!(
                        case,
                        ProposalCase::Limit | ProposalCase::TaskFence | ProposalCase::BindAbort
                    )
            )
        );
        let code = if policy == "deny" {
            "model_policy_denied"
        } else if policy == "org" {
            "model_org_approval_required"
        } else {
            "model_approval_required"
        };
        assert_eq!(reply.error_code.as_deref(), Some(code));
        let reason = reply
            .turn_metadata
            .as_ref()
            .unwrap()
            .failure_reason
            .as_ref()
            .unwrap();
        assert_eq!(
            (&*reason.source, &*reason.stage, &*reason.code),
            ("nyxid", "model_approval", code)
        );
        assert!(reply.text.contains("Approval settings"));
        assert!(
            reply.text.contains(
                &crate::services::assistant_links::AssistantPage::ApprovalSettings
                    .url(&state.config.frontend_url)
            )
        );
        assert_eq!(model_calls.load(Ordering::SeqCst), 0);
        if let Some(rows) = group_rows {
            assert!(
                rows.iter()
                    .any(|r| r.text.contains(code) && r.text.contains("/approvals/settings"))
            );
        }
    }
    if permissions
        && origin == "user"
        && policy == "write"
        && !matches!(
            case,
            ProposalCase::Limit | ProposalCase::TaskFence | ProposalCase::BindAbort
        )
    {
        assert_eq!(
            reply.activity_count,
            Some(1),
            "{case:?}: missing recovery evidence"
        );
        assert_eq!(reply.activities.len(), 1);
        let decision = reply.activities[0]
            .decision
            .as_ref()
            .expect("approval evidence");
        assert_eq!(decision.kind, "approval");
        assert_eq!(decision.state, "waiting");
        assert!(!decision.executing);
        assert!(
            state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"_id": &decision.id})
                .await
                .unwrap()
                .is_some()
        );
    }
    if policy != "deny" {
        assert_eq!(
            reply
                .turn_metadata
                .as_ref()
                .unwrap()
                .timing
                .as_ref()
                .unwrap()
                .model_calls,
            1
        );
    }
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(row) = state
                .db
                .collection::<AuditLog>("audit_log")
                .find_one(
                    doc! {"event_type":"service_request","event_data.user_service_id":&connection},
                )
                .await
                .unwrap()
            {
                break row.event_data.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(event["model_egress_admitted"], success);
    assert_eq!(
        event["outcome"],
        if success {
            "response_received"
        } else if policy == "deny" {
            "denied"
        } else {
            "approval_unsupported"
        }
    );
    if success {
        assert!(event.get("failure_code").is_none());
        assert_eq!(
            event.get("approval_exempt").and_then(Value::as_str),
            matches!(policy, "explicit" | "all").then_some("assistant_model_transport")
        );
    }
    assert!(!event.to_string().contains("CREW TEST OK"));
    assert!(!event.to_string().contains("test-only-master"));
    if case == ProposalCase::NextTurn {
        let first = state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        let prior = proposals::proposal(&first).unwrap();
        assert_eq!(first.status, "pending");
        callback_calls.store(0, Ordering::SeqCst);
        let input: engine::TurnRequest = serde_json::from_value(json!({
            "text":"Reply with only CREW TEST OK", "agent_id":nyxbot.id, "conversation_id":conversation.id, "retry_of":prior.source_turn_id
        })).unwrap();
        let mut input = TurnStart::from(&input);
        input.execution_enrollment = true;
        let retry = Box::pin(start_server_turn(
            &state,
            OWNER,
            input.clone(),
            Pool::Team {
                owner: OWNER,
                limit: 2,
            },
        ))
        .await;
        match retry {
            Err(AppError::Conflict(message)) => {
                assert!(
                    message.starts_with("Retry unavailable (activity_"),
                    "{message}"
                );
                eprintln!("pending model proposal retry: {message}");
            }
            Err(error) => panic!(
                "pending proposal must retain the existing recovery-evidence fence; got Err({error:?}): {error}"
            ),
            Ok(Started::Turn { .. }) => panic!(
                "pending proposal must retain the existing recovery-evidence fence; got Ok(Started::Turn)"
            ),
            Ok(Started::Busy) => panic!(
                "pending proposal must retain the existing recovery-evidence fence; got Ok(Started::Busy)"
            ),
            Ok(Started::PoolFull) => panic!(
                "pending proposal must retain the existing recovery-evidence fence; got Ok(Started::PoolFull)"
            ),
        }
        assert_eq!(callback_calls.load(Ordering::SeqCst), 0);
        assert!(
            engine::get(&state.db, OWNER, &conversation.id)
                .await
                .unwrap()
                .active_turn
                .is_none()
        );
        assert_eq!(
            state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        // A fresh owner message is a new turn and task, not a replay of the
        // unresolved effect. Preserve the base's occurrence-scoped gap key.
        input.retry_of = None;
        input.agent_id = None;
        let start = std::time::Instant::now();
        let Started::Turn {
            conversation: second,
            ..
        } = Box::pin(start_server_turn(
            &state,
            OWNER,
            input,
            Pool::Team {
                owner: OWNER,
                limit: 2,
            },
        ))
        .await
        .unwrap()
        else {
            panic!("not started");
        };
        tokio::time::timeout(Duration::from_secs(15), async {
            while engine::get(&state.db, OWNER, &second.id)
                .await
                .unwrap()
                .active_turn
                .is_some()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        settlement_signal.send_replace(());
        assert!(start.elapsed() < Duration::from_secs(15));
        let messages = engine::messages(&state.db, OWNER, &second.id, 20, None)
            .await
            .unwrap();
        let replies: Vec<_> = messages.iter().filter(|m| m.role == "assistant").collect();
        assert_eq!(replies.len(), 2);
        assert!(
            replies
                .iter()
                .all(|m| m.error_code.as_deref() == Some("model_approval_required"))
        );
        assert_eq!(callback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(model_calls.load(Ordering::SeqCst), 0);
        let approvals = state.db.collection::<ApprovalRequest>("approval_requests");
        assert_eq!(approvals.count_documents(doc! {}).await.unwrap(), 2);
        let next = approvals
            .find_one(doc! {"_id":{"$ne":&first.id}})
            .await
            .unwrap()
            .unwrap();
        let next = proposals::proposal(&next).unwrap();
        assert_ne!(next.task.task, prior.task.task);
        assert_ne!(next.source_turn_id, prior.source_turn_id);
        assert_ne!(next.gap_key, prior.gap_key);
        assert_eq!(
            approvals
                .find_one(doc! {"_id":&first.id})
                .await
                .unwrap()
                .unwrap()
                .status,
            "pending"
        );
    }
    agent_task.abort();
    model_task.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn model_approval_owner_personal_requirements_are_exempt() {
    for policy in ["explicit", "all", "manual_default"] {
        Box::pin(model_approval_pipeline("user", policy, false)).await;
    }
}
#[tokio::test]
async fn model_approval_crew_personal_requirements_are_exempt() {
    for policy in ["explicit", "all", "manual_default", "default"] {
        Box::pin(model_approval_pipeline("crew", policy, false)).await;
    }
}
#[tokio::test]
async fn model_approval_event_personal_requirements_are_exempt() {
    for policy in ["explicit", "all", "manual_default"] {
        Box::pin(model_approval_pipeline("event", policy, false)).await;
    }
}
#[tokio::test]
async fn model_approval_channel_personal_requirements_are_exempt() {
    for policy in ["explicit", "all", "manual_default"] {
        Box::pin(model_approval_pipeline("channel", policy, false)).await;
    }
}
#[tokio::test]
async fn model_approval_exemption_has_no_permission_proposals() {
    for origin in ["user", "crew", "event"] {
        for policy in ["explicit", "all"] {
            Box::pin(model_approval_pipeline(origin, policy, true)).await;
        }
    }
}
#[tokio::test]
async fn model_approval_invalid_shape_keeps_owner_wait_and_hidden_turn_fails_once() {
    for origin in ["user", "crew", "event"] {
        Box::pin(model_approval_pipeline(origin, "write", false)).await;
    }
    Box::pin(model_approval_pipeline("user", "write", true)).await;
}
#[tokio::test]
async fn model_approval_deny_is_prompt_once_without_dispatch() {
    for origin in ["user", "crew", "event"] {
        Box::pin(model_approval_pipeline(origin, "deny", false)).await;
    }
}

#[tokio::test]
async fn model_approval_invalid_shape_preserves_personal_full_access_behavior() {
    for origin in ["user", "crew", "event"] {
        Box::pin(model_approval_pipeline(origin, "write_full", false)).await;
    }
}

#[tokio::test]
async fn model_approval_org_policy_is_not_exempt_and_fails_promptly() {
    for origin in ["user", "crew", "event"] {
        Box::pin(model_approval_pipeline(origin, "org", false)).await;
    }
    Box::pin(model_approval_pipeline("crew", "org", true)).await;
}

#[tokio::test]
async fn model_approval_existing_proposal_still_settles_once() {
    Box::pin(model_approval_pipeline_case(
        "user",
        "write",
        true,
        ProposalCase::Existing,
    ))
    .await;
}

#[tokio::test]
async fn model_approval_proposal_limit_still_settles_once() {
    Box::pin(model_approval_pipeline_case(
        "user",
        "write",
        true,
        ProposalCase::Limit,
    ))
    .await;
}

#[tokio::test]
async fn model_approval_task_fence_failure_still_settles_once() {
    Box::pin(model_approval_pipeline_case(
        "user",
        "write",
        true,
        ProposalCase::TaskFence,
    ))
    .await;
}

#[tokio::test]
async fn model_approval_decided_proposal_still_settles_once() {
    Box::pin(model_approval_pipeline_case(
        "user",
        "write",
        true,
        ProposalCase::ExistingDecided,
    ))
    .await;
}

#[tokio::test]
async fn model_approval_pending_retry_and_new_turn_keep_gap_identity() {
    Box::pin(model_approval_pipeline_case(
        "user",
        "write",
        true,
        ProposalCase::NextTurn,
    ))
    .await;
}

#[tokio::test]
async fn model_approval_settlement_before_callback_retains_retry_evidence() {
    for case in [
        ProposalCase::NextTurn,
        ProposalCase::Existing,
        ProposalCase::ExistingDecided,
    ] {
        Box::pin(model_approval_pipeline_order(
            "user", "write", true, case, true,
        ))
        .await;
    }
}

#[tokio::test]
async fn model_approval_abort_after_binding_never_publishes_decision_flags() {
    Box::pin(model_approval_pipeline_case(
        "user",
        "write",
        true,
        ProposalCase::BindAbort,
    ))
    .await;
}
