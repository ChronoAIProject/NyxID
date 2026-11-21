use super::*;
use crate::handlers::{assistant_group, assistant_nyxagent, assistant_team};
use crate::models::assistant_conversation::{
    EngineWorkerInstance, RestartSource, TeamRestartSource,
};
use crate::services::{
    assistant_engine_worker as workers, assistant_group_service as groups,
    assistant_turn_restart as restart,
};
use std::time::Duration as Wait;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lane {
    Assignment,
    Member,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cut {
    None,
    BeforeCreated,
    MidAnswer,
    Stop,
    Lost,
    Remove,
    Delete,
    DeletePending,
    TextReport,
}

const SESSION: &str = "conv_11111111111111111111111111111111";
const RESPONSE: &str = "resp_11111111111111111111111111111111_22222222222222222222222222222222";

fn snapshot(status: &str) -> Value {
    json!({"status":status,"conversation_id":SESSION,"response_id":RESPONSE,"last_sequence_number":2,
        "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":if status=="lost" {"failed"}else{status},
        "error":(status=="lost").then(||json!({"code":"executor_lost","tool_dispatch":"none"})),
        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer @NyxBot"}]}]}})
}

#[derive(Default, Clone)]
struct Upstream {
    discovery: Requests,
    posts: Arc<AtomicUsize>,
    lookups: Arc<AtomicUsize>,
    cancels: Arc<AtomicUsize>,
    cancel_started: Arc<tokio::sync::Notify>,
    release_cancel: Arc<tokio::sync::Notify>,
    expected: Arc<Mutex<Option<(axum::http::HeaderValue, axum::http::HeaderValue)>>>,
}

impl Upstream {
    fn router(&self, cut: Cut, background: bool) -> Router {
        let posts = self.posts.clone();
        let captured = self.expected.clone();
        let expected = self.expected.clone();
        let lookups = self.lookups.clone();
        let cancels = self.cancels.clone();
        let cancel_started = self.cancel_started.clone();
        let release_cancel = self.release_cancel.clone();
        let cancel = post(move || {
            cancels.fetch_add(1, Ordering::SeqCst);
            let release_cancel = release_cancel.clone();
            cancel_started.notify_one();
            async move {
                if cut == Cut::DeletePending {
                    release_cancel.notified().await;
                    axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response()
                } else {
                    Json(snapshot("cancelled")).into_response()
                }
            }
        });
        self.discovery.router(false).route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let posts = posts.clone(); let captured = captured.clone(); async move {
                assert_eq!(body["background"] == true, background);
                posts.fetch_add(1, Ordering::SeqCst);
                if background { *captured.lock().await = Some((headers["authorization"].clone(),headers["idempotency-key"].clone())); }
                let stream = async_stream::stream! {
                    if cut != Cut::BeforeCreated {
                        let created = json!({"type":"response.created","sequence_number":0,"response":{"id":RESPONSE,"conversation":{"id":SESSION}}});
                        yield Ok::<_, Infallible>(Event::default().data(created.to_string()));
                        let delta = json!({"type":"response.output_text.delta","sequence_number":1,"delta":"ans"});
                        yield Ok(Event::default().data(delta.to_string()));
                    }
                    if cut != Cut::None { std::future::pending::<()>().await; }
                    let terminal = json!({"type":"response.completed","sequence_number":2,"response":snapshot("completed")["response"]});
                    yield Ok(Event::default().data(terminal.to_string()));
                };
                Sse::new(stream)
            }
        })).route("/v1/responses/lookup", axum::routing::get(move |headers: HeaderMap| {
            let expected = expected.clone(); let lookups = lookups.clone(); async move {
                let held = expected.lock().await;
                let (credential,key) = held.as_ref().expect("execution POST");
                assert!(headers["authorization"] == credential && headers["idempotency-key"] == key);
                lookups.fetch_add(1,Ordering::SeqCst);
                Json(snapshot(match cut { Cut::Lost => "lost", Cut::Stop => "running", _ => "completed" }))
            }
        })).route(&format!("/v1/responses/{RESPONSE}/events"), axum::routing::get(|| async {
            ([("content-type","text/event-stream")],axum::body::Body::from_stream(futures::stream::pending::<Result<axum::body::Bytes, Infallible>>()))
        })).route("/v1/responses/cancel", cancel.clone())
            .route(&format!("/v1/responses/{RESPONSE}/cancel"), cancel)
    }
}

async fn exercise(
    lane: Lane,
    cut: Cut,
    flag: bool,
    compatible: bool,
    warm: bool,
) -> std::collections::BTreeMap<String, usize> {
    let upstream = Upstream::default();
    let (fixture, _, server) =
        setup_with_response_router("team_handoff", upstream.router(cut, flag && compatible)).await;
    let (monitored, counts) = crate::services::durable_wake_tests::monitored(&fixture.db).await;
    let mut first = test_app_state(monitored);
    first.encryption_keys = fixture.encryption_keys.clone();
    let parent = engine::begin_turn(
        &fixture.db,
        OWNER,
        &engine::parse_turn(br#"{"text":"assign work"}"#).unwrap(),
        &fixture.encryption_keys,
    )
    .await
    .unwrap();
    let agent = specialist(&fixture).await;
    let group = if lane == Lane::Member {
        use crate::services::{
            assistant_settings_service as settings, coordination_service::RateWindowStore,
        };
        settings::update(
            &fixture.db,
            OWNER,
            settings::Update {
                max_group_handoffs_per_hour: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(
            RateWindowStore::admit(
                &fixture.db,
                "assistant_group_handoffs",
                OWNER,
                1,
                Wait::from_secs(3600)
            )
            .await
            .unwrap()
            .allowed
        );
        Some(
            groups::create(
                &fixture.db,
                OWNER,
                "Handoff crew",
                &[agent.id.clone(), parent.agent_id.clone().unwrap()],
                "user",
            )
            .await
            .unwrap(),
        )
    } else {
        None
    };
    readiness::observe_test_handoff_readiness(&fixture.db, true).await;
    if flag {
        set_flag(&fixture).await;
    }
    crate::services::assistant_notification_service::reset_flag_cache(&fixture.db);
    if !compatible {
        readiness::observe_v2_only_test_fleet(&fixture.db).await;
    }
    if warm {
        engine::require_enabled(&fixture.db, OWNER).await.unwrap();
        let metadata = assistant_nyxagent::restart_metadata(
            &fixture,
            &crate::test_utils::test_auth_user(OWNER),
            Some(SERVER_TURN_POLICY),
        )
        .await;
        assert!(metadata.deployment.is_some() && !metadata.models.is_empty());
    }
    let initial_discovery = upstream.discovery.counts();
    if cut == Cut::TextReport {
        fixture
            .db
            .collection::<Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":&parent.id},
                doc! {"$set":{"active_turn.stop_requested":true}},
            )
            .await
            .unwrap();
    }
    let previous = Box::leak(Box::new(EngineWorkerInstance {
        id: Uuid::new_v4().to_string(),
        hostname: None,
    }));
    counts.lock().unwrap().clear();
    let id = workers::TEST_WORKER
        .scope(previous, async {
            if let Some(group) = &group {
                assistant_group::post_and_follow(
                    &first,
                    OWNER,
                    &group.id,
                    &format!("@{} answer please", agent.name),
                    None,
                    &parent.id,
                )
                .await
                .unwrap();
                groups::member_thread(&fixture.db, OWNER, &group.id, &agent.id)
                    .await
                    .unwrap()
                    .expect("member admitted")
                    .id
            } else {
                let assistant_team::Started::Turn { conversation, .. } = assistant_team::assign(
                    &first,
                    OWNER,
                    &agent,
                    "answer please",
                    Some(&parent.id),
                    None,
                    None,
                )
                .await
                .unwrap() else {
                    panic!("assignment admission")
                };
                conversation.id
            }
        })
        .await;
    let collection = fixture
        .db
        .collection::<AssistantConversation>(CONVERSATIONS);
    let limit = Wait::from_secs(35);
    let mut measured = Default::default();
    let mut report_source = None;
    if cut != Cut::None {
        let row = tokio::time::timeout(limit, async {
            loop {
                let row = collection
                    .find_one(doc! {"_id":&id})
                    .await
                    .unwrap()
                    .unwrap();
                if upstream.posts.load(Ordering::SeqCst) == 1
                    && (cut == Cut::BeforeCreated
                        || row
                            .active_turn
                            .as_ref()
                            .and_then(|turn| turn.reply.as_ref())
                            .is_some_and(|reply| reply.text == "ans"))
                {
                    break row;
                }
                tokio::time::sleep(Wait::from_millis(25)).await;
            }
        })
        .await
        .expect("predecessor reached cut");
        let active = row.active_turn.as_ref().unwrap();
        if matches!(cut, Cut::Remove | Cut::Delete | Cut::DeletePending) {
            let group = group.as_ref().unwrap();
            assert_eq!(active.restart_admission.is_some(), flag);
            if cut == Cut::Remove {
                assistant_group::update_personal(
                    &fixture,
                    OWNER,
                    group,
                    None,
                    Some(std::slice::from_ref(parent.agent_id.as_ref().unwrap())),
                )
                .await
                .unwrap();
                assistant_group::advance_background(&fixture, OWNER, &group.id).await;
                assistant_group::advance_background(&fixture, OWNER, &group.id).await;
                let messages = groups::messages(&fixture.db, OWNER, &group.id, 50, None)
                    .await
                    .unwrap();
                assert_eq!(
                    messages
                        .iter()
                        .filter(|message| message.role == "notice"
                            && message.text == format!("{} stopped.", agent.name))
                        .count(),
                    1
                );
                assert!(!messages.iter().any(|message| message.role == "agent"));
                let target = engine::get(&fixture.db, OWNER, &parent.id).await.unwrap();
                assert_eq!(
                    target
                        .pending_events
                        .iter()
                        .filter(|event| event.kind == "group_settled")
                        .count(),
                    1
                );
            } else {
                if cut == Cut::DeletePending {
                    engine::request_stop_row_target(&fixture.db, &row)
                        .await
                        .unwrap();
                    let stopped = engine::get(&fixture.db, OWNER, &row.id).await.unwrap();
                    assert!(stopped.active_turn.is_none() && stopped.stopped_window.is_some());
                }
                let deleting = assistant_group::delete_group(
                    axum::extract::State(fixture.clone()),
                    crate::test_utils::test_auth_user(OWNER),
                    axum::extract::Path(group.id.clone()),
                );
                let inspect = async {
                    if cut == Cut::DeletePending {
                        tokio::time::timeout(limit, upstream.cancel_started.notified())
                            .await
                            .unwrap();
                        let stopped = collection
                            .find_one(doc! {"_id":&id})
                            .await
                            .unwrap()
                            .unwrap();
                        assert!(stopped.stopped_window.is_some());
                        assert!(
                            fixture
                                .db
                                .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
                                .find_one(doc! {"_id":&row.credential_api_key_id,"is_active":true})
                                .await
                                .unwrap()
                                .is_some()
                        );
                        upstream.release_cancel.notify_one();
                    }
                };
                let (response, ()) = tokio::join!(Box::pin(deleting), inspect);
                assert_eq!(response.unwrap(), axum::http::StatusCode::NO_CONTENT);
                assert!(matches!(
                    groups::get(&fixture.db, OWNER, &group.id).await,
                    Err(crate::errors::AppError::NotFound(_))
                ));
            }
            assert!(
                collection
                    .find_one(doc! {"_id":&id})
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                !fixture
                    .db
                    .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
                    .find_one(doc! {"_id":&row.credential_api_key_id})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_bool("is_active")
                    .unwrap()
            );
            assert_eq!(upstream.cancels.load(Ordering::SeqCst) > 0, flag);
            first.shutdown.stop();
            first.shutdown.tasks_stopped().await;
            server.abort();
            let _ = server.await;
            fixture.db.drop().await.unwrap();
            return measured;
        }
        if cut == Cut::TextReport {
            report_source = Some(row.clone());
        }
        let source = active.restart_admission.as_ref().unwrap();
        assert_eq!(source.version, 3);
        match source.team_source.as_ref().unwrap() {
            TeamRestartSource::Assignment { report_to } => {
                assert_eq!(report_to, &parent.id);
                if cut == Cut::TextReport {
                    assert!(active.delegated_images.assignment.is_none());
                } else {
                    assert_eq!(
                        active
                            .delegated_images
                            .assignment
                            .as_ref()
                            .unwrap()
                            .recipient_conversation,
                        *report_to
                    );
                }
                assert!(!active.with_nyxbot);
            }
            TeamRestartSource::GroupMember {
                group_id,
                upper_seq,
            } => {
                assert_eq!(group_id, &group.as_ref().unwrap().id);
                assert_eq!(*upper_seq, row.group_seen_seq);
                assert!(active.with_nyxbot);
            }
        }
        let legacy: LegacyAdmission =
            bson::from_document(bson::to_document(source.as_ref()).unwrap()).unwrap();
        assert_eq!(legacy.source, RestartSource::FirstPartyOwner);
        assert!(!matches!(legacy.version, 1 | 2));
        let team_limit =
            assistant_team::team_pool_limit(&first, OWNER).await + u32::from(lane == Lane::Member);
        let mut occupied = Vec::new();
        for _ in 1..team_limit {
            occupied.push(
                first
                    .direct_chat_limiter
                    .try_acquire_pool("assistant_team", OWNER, team_limit)
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        assert!(
            first
                .direct_chat_limiter
                .try_acquire_pool("assistant_team", OWNER, team_limit)
                .await
                .unwrap()
                .is_none()
        );
        if cut == Cut::BeforeCreated {
            first.shutdown.stop();
            first.shutdown.tasks_stopped().await;
            collection
                .update_one(
                    doc! {"_id": &id, "active_turn.worker_instance.id": &previous.id},
                    doc! {"$set": {"active_turn.heartbeat_at": bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            assert!(
                engine::sweep_orphaned_turn_rows(&fixture.db)
                    .await
                    .unwrap()
                    .is_empty()
            );
        } else {
            assert!(
                restart::offer(&fixture.db, &row, restart::Loss::Shutdown(&previous.id))
                    .await
                    .unwrap()
            );
        }
        let offered = restart::due(&fixture.db, &workers::instance().id)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert!(collection.find_one(doc!{"_id":&id,"active_turn.restart_admission.version":{"$in":[1,2]},"active_turn.restart_offer.claimed_by":null}).await.unwrap().is_none());
        assert!(
            assistant_nyxagent::survivor::take_one(&fixture, &offered, workers::instance())
                .await
                .unwrap()
        );
        assert!(
            !assistant_nyxagent::survivor::take_one(&fixture, &offered, workers::instance())
                .await
                .unwrap()
        );
        first.shutdown.stop();
        first.shutdown.tasks_stopped().await;
        if cut == Cut::Stop {
            tokio::time::timeout(limit, async {
                while upstream.lookups.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Wait::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            assistant_nyxagent::stop(
                axum::extract::State(fixture.clone()),
                crate::test_utils::test_auth_user(OWNER),
                axum::extract::Path(id.clone()),
            )
            .await
            .unwrap();
        }
        tokio::time::timeout(limit, fixture.shutdown.idle())
            .await
            .expect("survivor finished");
        drop(occupied);
    } else {
        tokio::time::timeout(limit, first.shutdown.idle())
            .await
            .expect("first replica finished");
        measured = crate::services::durable_wake_tests::take(&counts);
        let total: usize = measured.values().sum();
        eprintln!(
            "team_handoff_commands lane={lane:?} flag={flag} compatible={compatible} warm={warm} total={total} by_command={}",
            serde_json::to_string(&measured).unwrap()
        );
    }
    assert_eq!(
        upstream.discovery.counts(),
        if !warm && flag && compatible {
            (1, 1)
        } else {
            initial_discovery
        }
    );
    assert_eq!(upstream.posts.load(Ordering::SeqCst), 1);
    let replies = engine::messages(&fixture.db, OWNER, &id, 10, None)
        .await
        .unwrap();
    let mut replies = replies.iter().filter(|message| message.role == "assistant");
    let reply = replies.next().expect("saved reply");
    assert!(replies.next().is_none());
    match cut {
        Cut::Lost => assert_eq!(reply.error_code.as_deref(), Some("outcome_unknown")),
        Cut::Stop => assert_eq!(reply.error_code.as_deref(), Some("cancelled")),
        _ => assert_eq!(reply.text, "answer @NyxBot"),
    }
    if let Some(group) = group {
        assistant_group::advance_background(&fixture, OWNER, &group.id).await;
        let current = groups::get(&fixture.db, OWNER, &group.id).await.unwrap();
        assert_eq!(
            current.last_recipient_ids.as_slice(),
            std::slice::from_ref(&agent.id)
        );
        assert_eq!(
            current.hops_remaining,
            group.hops_remaining - i32::from(reply.error_code.is_none())
        );
        assert!(current.followers.is_empty());
        assert_eq!(
            fixture
                .db
                .collection::<Document>(crate::models::assistant_group::MESSAGES_COLLECTION_NAME)
                .count_documents(doc! {"group_id":&group.id,"agent_id":&agent.id})
                .await
                .unwrap(),
            1
        );
    } else {
        assert_eq!(reply.report_to.as_deref(), Some(parent.id.as_str()));
        if cut == Cut::TextReport {
            assert!(reply.delegated_images.assignment.is_none());
        } else {
            assert_eq!(
                reply
                    .delegated_images
                    .assignment
                    .as_ref()
                    .unwrap()
                    .assigning_turn,
                parent.active_turn.as_ref().unwrap().turn_id
            );
        }
    }
    let target = engine::get(&fixture.db, OWNER, &parent.id).await.unwrap();
    assert_eq!(
        target
            .pending_events
            .iter()
            .filter(|event| event.kind
                == if lane == Lane::Member {
                    "group_settled"
                } else {
                    "subagent_settled"
                })
            .count(),
        1
    );
    if let Some(source) = report_source {
        let enrolled = &target.pending_events[0];
        assert_eq!(enrolled.id, reply.turn_id);
        assert_eq!(enrolled.agent_id.as_deref(), Some(agent.id.as_str()));
        assert!(enrolled.delegated_result.is_none() && enrolled.reply_to.is_empty());
        assert!(enrolled.text.contains("Reply excerpt: \"answer @NyxBot\""));
        collection
            .update_one(doc! {"_id":&parent.id}, doc! {"$set":{"pending_events":[]}})
            .await
            .unwrap();
        Box::pin(assistant_team::after_turn(
            &fixture,
            &source,
            &reply.text,
            None,
        ))
        .await;
        assert!(
            engine::get(&fixture.db, OWNER, &parent.id)
                .await
                .unwrap()
                .pending_events
                .is_empty()
        );
        fixture
            .db
            .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&reply.id},
                doc! {"$unset":{"delegated_images.text_report_in_settlement":""}},
            )
            .await
            .unwrap();
        Box::pin(assistant_team::after_turn(
            &fixture,
            &source,
            &reply.text,
            None,
        ))
        .await;
        let legacy = engine::get(&fixture.db, OWNER, &parent.id).await.unwrap();
        assert_eq!(legacy.pending_events.len(), 1);
        assert_eq!(
            bson::to_document(enrolled).unwrap(),
            bson::to_document(&legacy.pending_events[0]).unwrap()
        );
    }
    server.abort();
    let _ = server.await;
    fixture.db.drop().await.unwrap();
    measured
}

#[derive(serde::Deserialize)]
struct LegacyAdmission {
    version: i32,
    source: RestartSource,
}

#[tokio::test]
async fn assigned_specialist_takeover_and_publication() {
    for cut in [Cut::BeforeCreated, Cut::MidAnswer, Cut::Stop, Cut::Lost] {
        exercise(Lane::Assignment, cut, true, true, false).await;
    }
}

#[tokio::test]
async fn personal_group_member_takeover_and_publication() {
    for cut in [Cut::BeforeCreated, Cut::MidAnswer, Cut::Stop, Cut::Lost] {
        exercise(Lane::Member, cut, true, true, false).await;
    }
}

#[tokio::test]
async fn team_lane_command_costs_and_rollout() {
    for lane in [Lane::Assignment, Lane::Member] {
        for warm in [false, true] {
            let off = exercise(lane, Cut::None, false, true, warm).await;
            let on = exercise(lane, Cut::None, true, true, warm).await;
            let mixed = exercise(lane, Cut::None, true, false, warm).await;
            assert_eq!(off, mixed);
            if warm {
                assert_eq!(off.values().sum::<usize>(), on.values().sum::<usize>());
            }
        }
    }
}

#[tokio::test]
async fn removed_group_member_stops_and_wakes_with_either_flag() {
    for flag in [false, true] {
        exercise(Lane::Member, Cut::Remove, flag, true, false).await;
    }
}

#[tokio::test]
async fn deleting_group_retires_running_and_unconfirmed_member_windows() {
    for cut in [Cut::Delete, Cut::DeletePending] {
        exercise(Lane::Member, cut, true, true, false).await;
    }
}

#[tokio::test]
async fn text_only_assignment_publishes_identical_event_once() {
    exercise(Lane::Assignment, Cut::TextReport, true, true, false).await;
}
