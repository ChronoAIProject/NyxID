use super::*;
use crate::models::assistant_group::AssistantGroup;
use crate::models::assistant_message::AssistantMessage;
use crate::services::assistant_group_service as groups;
use axum::{extract::Path, http::StatusCode, routing::get};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

const RESEARCH: &str = "Research finding: bread color alone cannot establish whole-grain content.";
const PLAN: &str = "Planning finding: use one thread with five numbered posts.";
const INSTRUCTION: &str = "Research whether darker bread means whole grain, plan and draft five posts, get an independent review, then publish the approved thread and read back its reply chain.";
const DRAFT_POSTS: [&str; 5] = [
    "Darker bread?",
    "Color varies.",
    "Read ingredients.",
    "Check whole-grain labels.",
    "Compare serving sizes.",
];
const PREVIOUS_POST: &str = "Previous publication on serving sizes";
const REVIEW: &str = "Independent review accepted all five claims for local publication.";
const PREFLIGHT: &str = "Publisher preflight read the previous thread successfully.";
const REVIEW_FAILED: &str = "nutrition-reviewer failed (model_unavailable).";

type Reply = Result<String, &'static str>;

struct PendingResponse {
    call: Call,
    release: oneshot::Sender<Reply>,
}

impl PendingResponse {
    fn reply(self, text: impl Into<String>) {
        self.release.send(Ok(text.into())).unwrap();
    }

    fn fail(self, code: &'static str) {
        self.release.send(Err(code)).unwrap();
    }

    fn contains(&self, facts: &[&str]) {
        let input = self.call.body["input"].as_str().unwrap();
        for fact in facts {
            assert!(
                input.contains(fact),
                "missing {fact:?} in upstream input {input:?}"
            );
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PostInput {
    text: String,
    reply_to: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct MockPost {
    id: String,
    #[serde(flatten)]
    input: PostInput,
}

type Posts = Arc<Mutex<Vec<MockPost>>>;

fn fake_workflow_upstream(calls: mpsc::UnboundedSender<PendingResponse>, posts: Posts) -> Router {
    let writes = posts.clone();
    Router::new()
        .route(
            "/v1/responses",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let calls = calls.clone();
                async move {
                    let (release, reply) = oneshot::channel();
                    calls.send(PendingResponse {
                        call: Call { authorization: headers["authorization"].to_str().unwrap().into(), body },
                        release,
                    }).unwrap_or_else(|_| panic!("script dropped the response receiver"));
                    let reply = reply.await.unwrap();
                    let session = format!("conv_{}", Uuid::new_v4().simple());
                    let response = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
                    let status = if reply.is_ok() { "completed" } else { "failed" };
                    let mut result = json!({"id": response, "conversation": {"id": session}, "status": status, "output": []});
                    match reply {
                        Ok(text) => result["output"] = json!([{
                            "type": "message", "role": "assistant",
                            "content": [{"type": "output_text", "text": text}],
                        }]),
                        Err(code) => result["error"] = json!({"code": code, "message": "Fixture model failure"}),
                    }
                    let event = json!({"type": format!("response.{status}"), "sequence_number": 1, "response": result});
                    Sse::new(futures::stream::iter([Ok::<_, Infallible>(Event::default().data(event.to_string()))]))
                }
            }),
        )
        .route("/posts", post(move |Json(input): Json<PostInput>| {
            let writes = writes.clone();
            async move {
                let mut posts = writes.lock().await;
                let published = MockPost { id: format!("local-{}", posts.len() + 1), input };
                posts.push(published.clone());
                Json(published)
            }
        }))
        .route("/posts/{id}", get(move |Path(id): Path<String>| {
            let posts = posts.clone();
            async move {
                posts.lock().await.iter().find(|post| post.id == id).cloned()
                    .map(Json).ok_or(StatusCode::NOT_FOUND)
            }
        }))
        .route("/previous-thread", get(|| async { Json(PREVIOUS_POST) }))
}

async fn next_call(
    state: &AppState,
    calls: &mut mpsc::UnboundedReceiver<PendingResponse>,
    stage: &str,
) -> PendingResponse {
    let timeout = std::time::Duration::from_secs(20);
    tokio::time::timeout(timeout, async {
        loop {
            tokio::select! {
                call = calls.recv() => return call.expect("response server stays connected"),
                () = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                    let (a, b) = tokio::join!(tick(state), tick(state));
                    a.unwrap();
                    b.unwrap();
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("workflow did not call NyxAgent for {stage}"))
}

async fn bound_call(state: &AppState, response: &PendingResponse) -> AssistantConversation {
    let raw = response.call.authorization.strip_prefix("Bearer ").unwrap();
    // Match against the credential of the active thread without emitting key
    // material in an assertion or in the test log.
    for row in state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"user_id": OWNER, "active_turn.automation_work": {"$exists": true}})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
    {
        let credential =
            crate::services::assistant_agent_credential_service::load_for_conversation(
                &state.db,
                &state.encryption_keys,
                OWNER,
                &row.id,
            )
            .await
            .unwrap()
            .unwrap();
        if credential.raw_key.as_str() == raw {
            return row;
        }
    }
    panic!("upstream request does not match a live automation thread key");
}

async fn post_group(
    state: &AppState,
    thread: &AssistantConversation,
    group: &AssistantGroup,
    text: &str,
    recipients: &[String],
) {
    let chat = crate::services::assistant_acknowledgement_service::for_key(
        &state.db,
        OWNER,
        Some(&thread.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let (result, error) = crate::handlers::assistant_team::execute_tool(
        state,
        &chat,
        "nyxid__post_to_group",
        &json!({"group": group.id, "text": text}),
    )
    .await;
    assert!(!error, "{result}");
    let mut addressed: Vec<String> =
        serde_json::from_value(result["addressed_agent_ids"].clone()).unwrap();
    addressed.sort();
    let mut expected = recipients.to_vec();
    expected.sort();
    assert_eq!(addressed, expected);
}

async fn waiting(state: &AppState, trigger: &Trigger, run_id: &str, continuations: u32) {
    let timeout = std::time::Duration::from_secs(10);
    let run = tokio::time::timeout(timeout, async {
        loop {
            let run = state
                .db
                .collection::<TriggerRun>(RUNS)
                .find_one(doc! {"_id": run_id})
                .await
                .unwrap()
                .unwrap();
            match run.outcome {
                RunOutcome::Waiting => break run,
                RunOutcome::Started => {}
                other => {
                    panic!("terminal or unexpected run outcome {other:?} before child release")
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(run.reason.as_deref(), Some("coordinator_pending"));
    assert!(run.finished_at.is_none());
    assert!(!run.terminal_result_recorded);
    assert_eq!(
        run.coordinator.as_ref().unwrap().continuations,
        continuations
    );
    let trigger_row = state
        .db
        .collection::<Document>(TRIGGERS)
        .find_one(doc! {"_id": &trigger.id})
        .await
        .unwrap()
        .unwrap();
    let schedule = trigger_row.get_document("schedule_state").unwrap();
    assert!(
        schedule
            .get_document("last_run")
            .ok()
            .is_none_or(|last| last.get_str("_id") != Ok(run_id))
    );
}

async fn coordinator_call(
    state: &AppState,
    calls: &mut mpsc::UnboundedReceiver<PendingResponse>,
    home: &AssistantConversation,
    run_id: &str,
    facts: &[&str],
) -> (PendingResponse, AssistantConversation) {
    let response = next_call(state, calls, "NyxBot home continuation").await;
    response.contains(facts);
    let row = bound_call(state, &response).await;
    assert_eq!(row.id, home.id);
    let turn = row.active_turn.as_ref().unwrap();
    assert_eq!(turn.trigger_run_id.as_deref(), Some(run_id));
    if !facts.is_empty() {
        assert!(turn.trigger_continuation);
        assert_eq!(turn.events.len(), 1);
        let event = &turn.events[0];
        assert_eq!(event.kind, "group_settled");
        assert_eq!(
            event.automation_work.as_ref().unwrap(),
            turn.automation_work.as_ref().unwrap()
        );
    }
    (response, row)
}

async fn publish_and_read_back(base: &str) -> (Vec<String>, String) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let mut previous = None;
    let mut links = Vec::new();
    for (index, text) in DRAFT_POSTS.iter().enumerate() {
        let input = PostInput {
            text: (*text).into(),
            reply_to: previous,
        };
        let post: MockPost = client
            .post(format!("{base}/posts"))
            .json(&input)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(post.id, format!("local-{}", index + 1));
        links.push(format!("{base}/posts/{}", post.id));
        previous = Some(post.id);
    }
    let mut previous = None;
    for (index, link) in links.iter().enumerate() {
        let read: MockPost = client
            .get(link)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(read.id, format!("local-{}", index + 1));
        assert_eq!(read.input.text, DRAFT_POSTS[index]);
        assert_eq!(read.input.reply_to, previous);
        previous = Some(read.id);
    }
    let verified = format!(
        "Verified five local posts and reply chain: {}",
        links.join(" ")
    );
    (links, verified)
}

async fn scheduled_workflow(editor_mentions: bool, review_fails: bool) {
    let (sender, mut calls) = mpsc::unbounded_channel();
    let posts: Posts = Arc::default();
    let upstream = fake_workflow_upstream(sender, posts.clone());
    let (mut state, base_url, server) =
        setup_with_response_router("scheduled_group_workflow", upstream).await;
    state.config.automation_continuation_v2_enabled = true;
    crate::services::security_readiness_service::ready_automation_test_replica(&state.db).await;
    let nyxbot = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let home = team::home_thread(&state.db, &state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    let researcher = named_specialist(&state, "researcher").await;
    let planner = named_specialist(&state, "planner").await;
    let editor = named_specialist(&state, "editor").await;
    let reviewer = named_specialist(&state, "nutrition-reviewer").await;
    let publisher = named_specialist(&state, "publisher").await;
    let group = groups::create(
        &state.db,
        OWNER,
        "Daily thread",
        &[
            nyxbot.id.clone(),
            researcher.id.clone(),
            planner.id.clone(),
            editor.id.clone(),
            reviewer.id.clone(),
            publisher.id.clone(),
        ],
        "user",
    )
    .await
    .unwrap();
    let trigger = automation_with_instruction(&state, &nyxbot.id, None, INSTRUCTION).await;
    let due = make_due(&state, &trigger).await;
    let run_id = schedules::run_id(&trigger.id, due);
    let (initial, active) = coordinator_call(&state, &mut calls, &home, &run_id, &[]).await;
    initial.contains(&["Started by trigger", INSTRUCTION]);
    post_group(
        &state,
        &active,
        &group,
        "@researcher find sources; @planner outline; @publisher read-only preflight",
        &[
            researcher.id.clone(),
            planner.id.clone(),
            publisher.id.clone(),
        ],
    )
    .await;

    let mut outstanding = Vec::new();
    for _ in 0..3 {
        let response = next_call(&state, &mut calls, "parallel early work").await;
        let row = bound_call(&state, &response).await;
        assert_eq!(row.group_id.as_deref(), Some(group.id.as_str()));
        assert_eq!(
            row.active_turn
                .as_ref()
                .unwrap()
                .automation_work
                .as_ref()
                .unwrap()
                .run_id,
            run_id
        );
        outstanding.push((row.agent_id.unwrap(), response));
    }
    assert_eq!(
        outstanding
            .iter()
            .map(|(id, _)| id)
            .collect::<std::collections::BTreeSet<_>>(),
        [&researcher.id, &planner.id, &publisher.id]
            .into_iter()
            .collect()
    );
    initial.reply("Early work dispatched");
    waiting(&state, &trigger, &run_id, 0).await;
    assert!(posts.lock().await.is_empty());

    // Emulate config-off only in the local scheduler instance. The admitted
    // occurrence and its group work have durable v2 enrollment.
    state.config.automation_continuation_v2_enabled = false;
    for (agent, response) in outstanding {
        if agent == researcher.id {
            response.reply(RESEARCH);
        } else if agent == planner.id {
            response.reply(PLAN);
        } else {
            response.contains(&["read-only preflight"]);
            let previous: String = reqwest::Client::new()
                .get(format!("{base_url}/previous-thread"))
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(previous, PREVIOUS_POST);
            response.reply(PREFLIGHT);
        }
    }
    let (next, active) = coordinator_call(
        &state,
        &mut calls,
        &home,
        &run_id,
        &[RESEARCH, PLAN, PREFLIGHT],
    )
    .await;
    post_group(
        &state,
        &active,
        &group,
        &format!("@editor write the five-post draft. {RESEARCH} {PLAN}"),
        std::slice::from_ref(&editor.id),
    )
    .await;
    let edit = next_call(&state, &mut calls, "editor").await;
    edit.contains(&[RESEARCH, PLAN]);
    assert_eq!(
        bound_call(&state, &edit).await.agent_id.as_deref(),
        Some(editor.id.as_str())
    );
    next.reply("Waiting for the editor");
    waiting(&state, &trigger, &run_id, 1).await;

    let draft = format!("Unique draft: {}", DRAFT_POSTS.join(" | "));
    if editor_mentions {
        edit.reply(format!("{draft}\n@nutrition-reviewer please review all five posts.\n@NyxBot the unique draft is ready."));
    } else {
        edit.reply(draft.clone());
        let (next, active) = coordinator_call(&state, &mut calls, &home, &run_id, &[&draft]).await;
        post_group(
            &state,
            &active,
            &group,
            &format!("@nutrition-reviewer review the five-post draft. {draft}"),
            std::slice::from_ref(&reviewer.id),
        )
        .await;
        next.reply("Waiting for independent review");
    }

    let mut review = None;
    for _ in 0..if editor_mentions { 2 } else { 1 } {
        let response = next_call(&state, &mut calls, "editor handoff").await;
        let row = bound_call(&state, &response).await;
        assert_eq!(row.group_id.as_deref(), Some(group.id.as_str()));
        response.contains(&[&draft]);
        if row.agent_id.as_deref() == Some(reviewer.id.as_str()) {
            assert!(review.replace(response).is_none());
        } else {
            assert!(editor_mentions);
            assert_eq!(row.agent_id.as_deref(), Some(nyxbot.id.as_str()));
            assert_ne!(row.id, home.id);
            response.reply("Group NyxBot acknowledged the editor draft.");
        }
    }
    let before_review = if editor_mentions { 1 } else { 2 };
    waiting(&state, &trigger, &run_id, before_review).await;
    // Drive another pair of scheduler ticks during the controlled review wait.
    let (one, two) = tokio::join!(tick(&state), tick(&state));
    one.unwrap();
    two.unwrap();
    waiting(&state, &trigger, &run_id, before_review).await;
    assert!(posts.lock().await.is_empty());
    assert!(matches!(
        calls.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    eprintln!(
        "run={run_id} editor_mentions={editor_mentions} Waiting with reviewer held and 0 publication writes"
    );

    let review = review.expect("the independent reviewer was invoked");
    let (terminal_outcome, terminal_reason, final_text) = if review_fails {
        review.fail("model_unavailable");
        let (next, _) =
            coordinator_call(&state, &mut calls, &home, &run_id, &[REVIEW_FAILED]).await;
        next.fail("model_unavailable");
        assert!(posts.lock().await.is_empty());
        (RunOutcome::Failed, "model_unavailable", None)
    } else {
        review.reply(format!("{REVIEW} {draft}"));
        let (next, active) =
            coordinator_call(&state, &mut calls, &home, &run_id, &[REVIEW, &draft]).await;
        post_group(
            &state,
            &active,
            &group,
            &format!("@publisher formally publish the reviewed five posts and read back the chain. {REVIEW} {draft}"),
            std::slice::from_ref(&publisher.id),
        )
        .await;
        let publication = next_call(&state, &mut calls, "formal publisher").await;
        let target = bound_call(&state, &publication).await;
        assert_eq!(target.agent_id.as_deref(), Some(publisher.id.as_str()));
        publication.contains(&["formally publish the reviewed five posts", REVIEW, &draft]);
        next.reply("Waiting for publication and readback");
        waiting(&state, &trigger, &run_id, before_review + 1).await;
        let (links, verified) = publish_and_read_back(&base_url).await;
        eprintln!("run={run_id} fixture read back five IDs and reply edges {links:?}");
        publication.reply(verified.clone());
        let (last, _) = coordinator_call(&state, &mut calls, &home, &run_id, &[&verified]).await;
        last.reply(verified.clone());
        assert_eq!(posts.lock().await.len(), 5);
        (
            RunOutcome::Completed,
            "coordinator_completed",
            Some(verified),
        )
    };
    let finished = completed(&state, &trigger).await;
    assert_eq!(finished.id, run_id);
    assert_eq!(finished.outcome, terminal_outcome);
    assert_eq!(finished.reason.as_deref(), Some(terminal_reason));
    assert_eq!(finished.thread_id.as_deref(), Some(home.id.as_str()));
    assert!(finished.terminal_result_recorded);
    let coordinator = finished.coordinator.as_ref().unwrap();
    assert_eq!(coordinator.protocol, 2);
    assert_eq!(
        coordinator.dispatches,
        5 + u32::from(editor_mentions) + u32::from(!review_fails)
    );
    assert_eq!(
        coordinator.continuations,
        before_review + if review_fails { 1 } else { 2 }
    );
    let final_reply = state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
        "user_id": OWNER, "conversation_id": &home.id, "turn_id": &finished.turn_id, "role": "assistant",
    }).await.unwrap().unwrap();
    if let Some(text) = final_text {
        assert_eq!(final_reply.text, text);
        assert_eq!(final_reply.error_code, None);
    } else {
        assert_eq!(final_reply.error_code.as_deref(), Some(terminal_reason));
    }
    assert!(
        groups::get(&state.db, OWNER, &group.id)
            .await
            .unwrap()
            .followers
            .is_empty()
    );
    assert!(
        engine::get(&state.db, OWNER, &home.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    let budget = state
        .db
        .collection::<Document>(schedules::BUDGETS)
        .find_one(doc! {"_id": OWNER})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(budget.get_i32("hourly"), Ok(1));
    assert_eq!(budget.get_i32("daily"), Ok(1));
    assert_eq!(
        state
            .db
            .collection::<TriggerRun>(RUNS)
            .count_documents(doc! {"trigger_id": &trigger.id})
            .await
            .unwrap(),
        1
    );
    server.abort();
}

#[tokio::test]
async fn scheduled_group_resumes_without_a_mention_and_publishes_after_review() {
    scheduled_workflow(false, false).await;
}

#[tokio::test]
async fn scheduled_group_routes_editor_mentions_and_publishes_after_review() {
    scheduled_workflow(true, false).await;
}

#[tokio::test]
async fn scheduled_group_review_failure_propagates_without_publishing() {
    scheduled_workflow(false, true).await;
}
