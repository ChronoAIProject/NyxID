//! Enrolled owner ingress, real execution choices and typed retained dispatch.
use super::*;
use crate::services::execution_permission_dispatch::{phase_b_tests, proposals};

#[test]
fn supervisor_write_retained_readback_missing_or_mismatched_target_cannot_verify_selected_row() {
    assert!(retained_readback_target("Independently read this exact record").is_none());
    let input = "NyxID has settled the retained write attempt for Base baseOther, table tableOther, record recControl. Independently read this exact record";
    let (base, table, record) = retained_readback_target(input).unwrap();
    let result = json!({"content":[{"type":"text","text":json!({"code":0,"data":{"items":[
        {"record_id":"recControl","fields":{"Status":"Untouched","Observation":"provider-only:recControl"}}
    ]}}).to_string()}]});
    let observation = row_observation_at(&result, &base, &table, &record).unwrap();
    assert_eq!(
        checked_report(Some(&observation), "recSelected"),
        NO_CONFIRMATION
    );
    assert!(row_observation_at(&result, &base, &table, "recMissing").is_none());
}

async fn marked_journey(proxy: bool) -> Journey {
    marked_transport(proxy, "relay").await
}
async fn marked_transport(proxy: bool, transport: &str) -> Journey {
    let mut j = setup_transport_journey("api-feishu", proxy, transport).await;
    j.script.marked.store(true, Ordering::SeqCst);
    phase_b_tests::enroll_global(&j.state.db, OWNER).await;
    // Own the production refresh/watch loop for the whole journey, including
    // multi-case tests that outlast the snapshot TTL. Dropping the handle aborts
    // Cache.run and its owned change-stream JoinSet with the isolated fixture.
    let snapshot = j.state.execution_enrollment.clone();
    snapshot.refresh(&j.state.db).await.unwrap();
    let db = j.state.db.clone();
    j._enrollment_snapshot_worker = Some(tokio_util::task::AbortOnDropHandle::new(tokio::spawn(
        async move { snapshot.run(db).await },
    )));
    j._readiness_heartbeat = Some(tokio_util::task::AbortOnDropHandle::new(
        crate::services::security_readiness_service::controlled_ready_test_replica(&j.state.db)
            .await,
    ));
    assert_eq!(
        j.state
            .db
            .collection::<Document>("service_approval_configs")
            .count_documents(doc! {"user_id":OWNER})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        crate::services::approval_service::user_approval_mode(&j.state.db, OWNER)
            .await
            .unwrap(),
        crate::services::approval_policy::GlobalApprovalMode::WritesOnly
    );
    j
}

async fn marked_owner(j: &Journey, text: &str, id: &str) -> Response {
    assert!(
        crate::services::execution_permission_enrollment::snapshot::channel_candidate(
            &j.state.db,
            OWNER
        ),
        "the marked fixture must present a fresh real enrollment hint"
    );
    if j.channel.platform == "discord" {
        let provider_id = u64::from_str_radix(&sha256_hex(id.as_bytes())[..16], 16)
            .unwrap()
            .to_string();
        native_owner(j, text, &provider_id).await
    } else {
        owner(j, text, id).await
    }
}

async fn marked_assign(j: &Journey, text: &str, id: &str) -> tokio::task::JoinHandle<String> {
    let card = owner_text(&body_text(marked_owner(j, text, &format!("{id}-ask")).await).await);
    if !regex::Regex::new(r"yes ([0-9]{4})")
        .unwrap()
        .is_match(&card)
    {
        panic!(
            "a fresh assignment needs its own owner-facing code: {}",
            review_diagnostics(j).await
        );
    }
    let sent = delegation_review(&card).expect("the actual owner-facing delegation card");
    let card_id = sent["acknowledgement_id"].as_str().unwrap().to_owned();
    let conversation = sent["conversation_id"].as_str().unwrap().to_owned();
    let source = j
        .state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .find_one(doc! {"_id":&card_id,"user_id":OWNER,"conversation_id":&conversation})
        .projection(doc! {"_id":1,"status":1,"requested_turn_id":1,"execution_task.task.root_id":1})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.get_str("status").unwrap(), "pending");
    let original_root = source
        .get_document("execution_task")
        .and_then(|task| task.get_document("task"))
        .and_then(|task| task.get_str("root_id"))
        .unwrap_or_else(|_| panic!("marked card {card_id} must retain its admitted channel task"))
        .to_owned();
    let code = delegation_code(&card);
    let response = marked_owner(j, &format!("yes {code}"), &format!("{id}-assign")).await;
    let state = j.state.clone();
    let observed = j.observed.clone();
    tokio::spawn(async move {
        let text = body_text(response).await;
        let used = state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .find_one(doc! {"_id":&card_id,"user_id":OWNER,"conversation_id":&conversation})
            .projection(
                doc! {"_id":1,"status":1,"requested_turn_id":1,"execution_task.task.root_id":1},
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            used.get_str("status").unwrap(),
            "used",
            "the reply must consume the exact sent delegation card"
        );
        assert_eq!(
            used.get("requested_turn_id"),
            source.get("requested_turn_id")
        );
        let mut observations = observed.lock().await;
        let retry = observations
            .iter()
            .find(|event| {
                event["stage"] == "delegation_retry"
                    && event["acknowledgement_id"] == card_id
                    && event["conversation_id"] == conversation
            })
            .expect("the product input must name the exact reviewed card");
        let reply_turn = retry["turn_id"].clone();
        let reply_root = retry["task_root"].clone();
        observations.push(json!({"stage":"delegation_card_used","acknowledgement_id":card_id,
            "conversation_id":conversation,"requested_turn_id":source.get_str("requested_turn_id").unwrap(),
            "original_root":original_root,
            "reply_turn":reply_turn,"reply_root":reply_root}));
        text
    })
}

#[tokio::test]
async fn supervisor_write_marked_fresh_request_uses_its_card_despite_an_older_allowed_card() {
    let j = Box::pin(marked_journey(false)).await;
    let request = "Ask accountant for today's pending ledger rows";
    let old = owner_text(&body_text(marked_owner(&j, request, "ws7-stale-card-first").await).await);
    let old = delegation_review(&old).unwrap();
    let old_id = old["acknowledgement_id"].as_str().unwrap();
    let conversation = old["conversation_id"].as_str().unwrap();
    // An actual owner decision in the ordinary card service leaves this older
    // action allowed. A later new request is not a retry of that decision.
    let allowed = crate::services::assistant_acknowledgement_service::decide(
        &j.state.db,
        OWNER,
        conversation,
        old_id,
        true,
    )
    .await
    .unwrap();
    assert_eq!(allowed.status, "allowed");
    let fresh =
        owner_text(&body_text(marked_owner(&j, request, "ws7-stale-card-fresh").await).await);
    let fresh =
        delegation_review(&fresh).expect("a fresh owner request must get its own pending card");
    let fresh_id = fresh["acknowledgement_id"].as_str().unwrap();
    assert_ne!(fresh_id, old_id);
    assert_eq!(fresh["conversation_id"], conversation);
    let cards = j
        .state
        .db
        .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
            "assistant_acknowledgements",
        );
    let pending = cards
        .find_one(doc! {"_id":fresh_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, "pending");
    assert_eq!(
        cards
            .find_one(doc! {"_id":old_id})
            .await
            .unwrap()
            .unwrap()
            .status,
        "allowed"
    );
    assert!(
        !j.observed
            .lock()
            .await
            .iter()
            .any(|e| e["stage"] == "delegation_retry")
    );
    assert!(j.ledger.lock().unwrap().reads.is_empty());
    assert_no_write(&j);
    let phrase = fresh["confirm_phrase"].as_str().unwrap();
    let response = marked_owner(&j, phrase, "ws7-stale-card-fresh-yes").await;
    let report = owner_text(&body_text(response).await);
    assert_row_report(&report, &j, "Pending", 1);
    assert_eq!(
        cards
            .find_one(doc! {"_id":fresh_id})
            .await
            .unwrap()
            .unwrap()
            .status,
        "used"
    );
    assert_eq!(
        cards
            .find_one(doc! {"_id":old_id})
            .await
            .unwrap()
            .unwrap()
            .status,
        "allowed"
    );
    let evidence = j.observed.lock().await;
    let retry = evidence
        .iter()
        .find(|e| e["stage"] == "delegation_retry")
        .unwrap();
    assert_eq!(retry["acknowledgement_id"], fresh_id);
    assert_ne!(
        retry["turn_id"],
        pending.requested_turn_id.as_deref().unwrap()
    );
    assert_ne!(
        retry["task_root"],
        pending.execution_task.as_ref().unwrap().task.root_id
    );
    drop(evidence);
    assert_no_write(&j);
    remove_adapter(&j);
}

fn assert_no_write(j: &Journey) {
    let ledger = j.ledger.lock().unwrap();
    assert!(ledger.writes.is_empty());
    assert_eq!(ledger.target, "Pending");
    assert_eq!(ledger.control, "Untouched");
}

async fn load(j: &Journey, id: &str) -> ApprovalRequest {
    j.state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

async fn wait_proposal(j: &Journey) -> ApprovalRequest {
    let waited = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(row) = j
                .state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"user_id":OWNER,"execution_proposal.version":5,
                "resource":{"$regex":format!("/{}$",j.script.selected)}})
                .await
                .unwrap()
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    match waited {
        Ok(row) => row,
        Err(_) => panic!("marked proposal missing: {}", review_diagnostics(j).await),
    }
}

async fn wait_terminal(j: &Journey, id: &str) -> ApprovalRequest {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let row = load(j, id).await;
            if matches!(
                proposals::proposal(&row).unwrap().state.as_str(),
                "completed" | "not_sent" | "unknown" | "cancelled"
            ) {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the normal retained worker must settle its exact occurrence")
}

fn choice(row: &ApprovalRequest, name: &str, key: &str) -> proposals::Decision {
    let p = proposals::proposal(row).unwrap();
    let selected = p.choices.iter().find(|c| c.id == name).unwrap();
    proposals::Decision {
        approved: true,
        expected_revision: p.revision,
        choice_id: Some(selected.id.clone()),
        choice_digest: Some(selected.digest.clone()),
        idempotency_key: key.into(),
    }
}

async fn web_allow(j: &Journey, row: &ApprovalRequest, name: &str, key: &str) {
    let Json(response) = crate::handlers::approvals::decide_execution_request(
        State(j.state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(row.id.clone()),
        Json(choice(row, name, key)),
    )
    .await
    .unwrap();
    assert_eq!(response.id, row.id);
    assert_eq!(response.status, "approved");
    assert_eq!(response.decision_channel.as_deref(), Some("web"));
    assert!(
        response
            .execution_proposal
            .as_ref()
            .is_some_and(
                |p| p.version == row.execution_proposal.as_ref().unwrap().version && p.supported
            )
    );
}

async fn reject_ordinary_web(j: &Journey, review: &ApprovalRequest) {
    assert!(
        crate::handlers::approvals::decide_request(
            State(j.state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(review.id.clone()),
            crate::telemetry::TelemetryContext::default(),
            HeaderMap::new(),
            Json(crate::handlers::approvals::DecideRequest {
                approved: true,
                duration_sec: None
            }),
        )
        .await
        .is_err()
    );
    assert_ne!(load(j, &review.id).await.status, "approved");
    assert_no_write(j);
}

struct DeliveryCursor {
    requests: usize,
    observations: usize,
}
async fn cursor(j: &Journey) -> DeliveryCursor {
    DeliveryCursor {
        requests: j.chat.received_requests().await.unwrap().len(),
        observations: j.observed.lock().await.len(),
    }
}

async fn confirmed_dm_report(
    j: &Journey,
    cursor: &DeliveryCursor,
    turn: &str,
    message: &Value,
    task: &crate::models::execution_permission::TaskAdmission,
    progress: &mut Value,
) -> Option<String> {
    use crate::models::{
        channel_reply_delivery::{ChannelReplyDelivery, PartStatus},
        execution_permission_channel::Delivery,
    };
    use crate::services::{channel_platform::OutboundReply, channel_reply_plan as plan};

    let source = task.channel_source.as_ref().unwrap();
    let delivery = j
        .state
        .db
        .collection::<Delivery>("execution_permission_deliveries")
        .find_one(doc! {
            "user_id":OWNER,"conversation_id":&source.conversation_id,
            "turn_id":turn,"message_id":message["id"].as_str().unwrap(),
            "task.task.root_id":&task.task.root_id,"task.epoch":task.epoch,
            "task.channel_source.event_id":&source.event_id
        })
        .await
        .unwrap()?;
    *progress = json!({"owner_turn":turn,"delivery":delivery.id,"state":delivery.state,
        "parts":delivery.preparation.as_ref().map(|p| (p.next,p.total,p.receipts.len()))});
    if delivery.state != "confirmed" {
        return None;
    }
    assert_eq!(j.channel.platform, "telegram");
    assert!(j.channel.gateway_source.is_none());
    assert_eq!(message["status"], "completed");
    assert!(message["attachments"].as_array().unwrap().is_empty());
    let parts = plan::ordinary(
        &OutboundReply {
            text: Some(message["text"].as_str().unwrap().to_owned()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            metadata: plan::assistant_metadata("telegram"),
        },
        "telegram",
        "123",
    )
    .unwrap()
    .unwrap();
    assert!(!parts.is_empty());
    let prepared = delivery.preparation.as_ref().unwrap();
    assert_eq!(prepared.fingerprint, plan::fingerprint(&parts));
    assert_eq!(prepared.total as usize, parts.len());
    assert_eq!(prepared.next, prepared.total);
    assert_eq!(prepared.receipts.len(), parts.len());

    // Bind every confirmed transport receipt to the original source. Provider
    // message IDs alone are insufficient: the mock deliberately reuses ID 91.
    let event = j
        .state
        .db
        .collection::<Document>("nyxbot_events")
        .find_one(doc! {"_id":&source.event_id,"user_id":OWNER,"channel_id":&j.channel.id})
        .projection(doc! {"relay_message_id":1})
        .await
        .unwrap()
        .unwrap();
    let anchor = event.get_str("relay_message_id").unwrap();
    let original = j
        .state
        .db
        .collection::<Document>("channel_messages")
        .find_one(
            doc! {"_id":anchor,"user_id":OWNER,"channel_bot_id":&j.channel.channel_bot_id,
            "conversation_id":&j.channel.route_id,"direction":"inbound","platform":"telegram",
            "platform_conversation_id":"123","sender_platform_id":"7","original_message":true,
            "thread_id":null},
        )
        .projection(doc! {"platform_message_id":1})
        .await
        .unwrap()
        .expect("the result must use the exact verified owner's original DM source");
    let reply_to = original
        .get_str("platform_message_id")
        .unwrap()
        .parse::<i64>()
        .ok();
    for receipt in &prepared.receipts {
        let outbound = receipt.outbound_message_id.as_deref().unwrap();
        assert_eq!(receipt.platform_message_ids.len(), 1);
        let count = j
            .state
            .db
            .collection::<Document>("channel_messages")
            .count_documents(doc! {
                "_id":outbound,"user_id":OWNER,"channel_bot_id":&j.channel.channel_bot_id,
                "conversation_id":&j.channel.route_id,"direction":"outbound","platform":"telegram",
                "platform_conversation_id":"123","reply_to_message_id":anchor,
                "platform_message_id":&receipt.platform_message_ids[0]
            })
            .await
            .unwrap();
        assert_eq!(count, 1, "each result part must retain its original source");
        let transport = j
            .state
            .db
            .collection::<ChannelReplyDelivery>("channel_reply_deliveries")
            .find_one(
                doc! {"user_id":OWNER,"channel_bot_id":&j.channel.channel_bot_id,
                "inbound_message_id":anchor,"parts.outbound_message_id":outbound},
            )
            .await
            .unwrap()
            .unwrap();
        assert!(transport.complete);
        assert_eq!(transport.expected_parts, 1);
        assert_eq!(transport.parts.len(), 1);
        assert!(transport.parts[0].status == PartStatus::Accepted);
        assert_eq!(
            transport.parts[0].platform_message_ids,
            receipt.platform_message_ids
        );
        assert!(transport.parts[0].formatting_fallback.is_none());
    }

    // Telegram receives the complete rendered plan, including entities and the
    // source reply anchor when numeric. Compare whole bodies for every part;
    // marker-only matching could hide a missing denial or rejection notice.
    let expected: Vec<Value> = parts
        .iter()
        .map(|part| {
            let mut body = json!({"chat_id":"123","text":part.text.as_deref().unwrap()});
            if let Some(entities) = part.metadata.as_ref().and_then(|m| m.get("entities")) {
                body["entities"] = entities.clone();
            } else {
                body["parse_mode"] = json!("Markdown");
            }
            if let Some(id) = reply_to {
                body["reply_to_message_id"] = json!(id);
            }
            body
        })
        .collect();
    let received: Vec<Value> = j
        .chat
        .received_requests()
        .await
        .unwrap()
        .iter()
        .skip(cursor.requests)
        .filter(|request| request.url.path().ends_with("/sendMessage"))
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();
    progress["later_sends"] = json!(received.len());
    received
        .windows(expected.len())
        .find(|window| *window == expected.as_slice())
        .map(|bodies| {
            bodies
                .iter()
                .map(|body| body["text"].as_str().unwrap())
                .collect::<Vec<_>>()
                .join("")
        })
}

async fn later_dm_report(
    j: &Journey,
    cursor: &DeliveryCursor,
    review: &ApprovalRequest,
    marker: &str,
) -> String {
    let binding = review.supervisor_write.as_ref().unwrap();
    let task = &binding.marked.as_ref().unwrap().task;
    assert_eq!(binding.source.channel.channel_id, j.channel.id);
    let mut progress = json!({"phase":"matching_owner_event"});
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let owner_turn = j
                .observed
                .lock()
                .await
                .iter()
                .skip(cursor.observations)
                .find(|event| {
                    event["stage"] == "supervisor_event"
                        && event["conversation_id"] == binding.source.channel.conversation_id
                        && event["sources"].as_array().unwrap().iter().any(|source| {
                            source["id"] == binding.specialist_turn
                                && source["agent_id"] == binding.specialist_agent
                                && source["task_root"] == task.task.root_id
                                && source["task_epoch"] == task.epoch
                        })
                })
                .and_then(|event| event["turn_id"].as_str().map(str::to_owned));
            if let Some(turn) = owner_turn {
                let history = owner_history(j, &binding.source.channel.conversation_id).await;
                progress = json!({"owner_turn":turn,"phase":"matching_saved_result"});
                if let Some(message) = history["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["role"] == "assistant" && m["turn_id"] == turn)
                    .filter(|m| m["text"].as_str().is_some_and(|text| text.contains(marker)))
                    && let Some(report) =
                        confirmed_dm_report(j, cursor, &turn, message, task, &mut progress).await
                {
                    j.settlements.lock().await.wait(&turn).await;
                    return report;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    match result {
        Ok(report) => report,
        Err(_) => panic!(
            "a later complete rendered send must match specialist turn {}, agent {}, task {} and owner {}: progress={progress}; {}",
            binding.specialist_turn,
            binding.specialist_agent,
            task.task.root_id,
            binding.source.channel.conversation_id,
            review_diagnostics(j).await
        ),
    }
}

async fn reject_other_routes(j: &Journey, review: &ApprovalRequest, code: &str) {
    let mut wrong = event(&format!("yes write-{code}"), "8", "ws7-marked-wrong-sender");
    wrong["event_context"]["activity"]["conversation"]["id"] = json!("123");
    let _ = body_text(respond(&j.state, &j.key, &wrong, "ws7-marked-wrong-sender").await).await;
    assert_eq!(load(j, &review.id).await.status, "pending");
    assert_no_write(j);
    let other = "conv_22222222222222222222222222222222";
    assert_eq!(
        put_conversation(
            State(j.state.clone()),
            Path(("bnd_ws7".into(), other.into())),
            bearer(&j.key)
        )
        .await
        .status(),
        StatusCode::OK
    );
    let mut wrong = event(&format!("yes write-{code}"), "7", "ws7-marked-wrong-chat");
    wrong["conversation"] = json!(other);
    wrong["event_context"]["conversation_id"] = json!(other);
    wrong["event_context"]["activity"]["conversation"]["id"] = json!("456");
    let _ = body_text(respond(&j.state, &j.key, &wrong, "ws7-marked-wrong-chat").await).await;
    let current = load(j, &review.id).await;
    assert_eq!(current.status, "pending");
    assert!(
        current
            .supervisor_write
            .as_ref()
            .unwrap()
            .dm_decision_receipt
            .is_none()
    );
    assert_eq!(
        current
            .supervisor_write
            .as_ref()
            .unwrap()
            .source
            .channel
            .event_id,
        review
            .supervisor_write
            .as_ref()
            .unwrap()
            .source
            .channel
            .event_id
    );
    assert_no_write(j);
}

async fn initial_gap(j: &Journey) -> ApprovalRequest {
    initial_gap_stage(j, 1).await
}
async fn initial_gap_stage(j: &Journey, stage: usize) -> ApprovalRequest {
    let listing = marked_assign(
        j,
        "Ask accountant for today's pending ledger rows",
        "ws7-marked-list",
    )
    .await;
    let list = owner_text(
        &tokio::time::timeout(Duration::from_secs(25), listing)
            .await
            .unwrap()
            .unwrap(),
    );
    assert_row_report(&list, j, "Pending", 1);
    assert!(list.contains(&format!("record {}: Status = Untouched", j.script.control)));
    assert_no_write(j);
    j.stage.store(stage, Ordering::SeqCst);
    let result = marked_assign(j, &j.script.selection(), "ws7-marked-write").await;
    let initial = owner_text(
        &tokio::time::timeout(Duration::from_secs(25), result)
            .await
            .unwrap()
            .unwrap(),
    );
    assert!(
        initial.contains(NO_CONFIRMATION),
        "a pending write cannot confirm a final value"
    );
    let row = wait_proposal(j).await;
    let p = proposals::proposal(&row).unwrap();
    assert_eq!(row.status, "pending");
    assert!(p.task.channel_source.is_some());
    assert!(p.report_to.is_some());
    assert!(matches!(
        &p.contract.subject.principal,
        crate::models::execution_permission::Principal::Agent(_)
    ));
    assert_no_write(j);
    assert_eq!(
        j.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {"supervisor_write":{"$ne":null}})
            .await
            .unwrap(),
        0,
        "the web-only gap cannot be answered with a DM decision"
    );
    let _ = body_text(marked_owner(j, "yes write-000000000000", "ws7-marked-not-web").await).await;
    assert_eq!(load(j, &row.id).await.status, "pending");
    row
}

fn remove_adapter(j: &Journey) {
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&j.channel.channel_bot_id);
}

#[tokio::test]
async fn supervisor_write_marked_rejection_survives_readback_recovery_and_later_success() {
    for (proxy, already_matches) in [(false, false), (true, true)] {
        let j = Box::pin(marked_journey(proxy)).await;
        let proposal = initial_gap(&j).await;
        j.script.reject_write.store(true, Ordering::SeqCst);
        j.script.hold_readback.store(true, Ordering::SeqCst);
        if already_matches {
            j.ledger.lock().unwrap().target = "Approved".into();
        }
        web_allow(&j, &proposal, "persistent", &Uuid::new_v4().to_string()).await;
        let (review, code) = wait_review(&j).await;
        let delivery = cursor(&j).await;
        let _ = body_text(owner(&j, &format!("yes write-{code}"), "ws7-provider-reject-yes").await)
            .await;
        let completed = wait_terminal(&j, &proposal.id).await;
        assert_eq!(completed.activity_outcome.as_deref(), Some("failed"));
        let p = proposals::proposal(&completed).unwrap();
        assert_eq!(p.state, "completed");
        assert_eq!(p.failure_code.as_deref(), Some("provider_http_rejected"));
        tokio::time::timeout(Duration::from_secs(15), async {
            while !j
                .observed
                .lock()
                .await
                .iter()
                .any(|e| e["stage"] == "readback_started")
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let row = engine::get(&j.state.db, OWNER, &p.conversation_id)
            .await
            .unwrap();
        let egress = crate::handlers::assistant_nyxagent::http_egress(
            j.state.clone(),
            crate::test_utils::test_auth_user(OWNER),
        );
        for prior_code in [true, false] {
            if !prior_code {
                // A previously stored failed receipt may predate the explicit
                // rejection code; its exact request still records failed activity.
                j.state
                    .db
                    .collection::<Document>("approval_requests")
                    .update_one(
                        doc! {"_id":&proposal.id},
                        doc! {"$set":{"execution_proposal.v4.failure_code":null}},
                    )
                    .await
                    .unwrap();
            }
            let recovered = proposals::resume_turn_with_readback(&j.state, egress.as_ref(), &row)
                .await
                .unwrap()
                .unwrap();
            assert!(recovered.text.contains("provider rejected"));
            assert!(recovered.specialist_readback.is_none());
            assert_eq!(j.ledger.lock().unwrap().writes.len(), 1);
        }
        j.script.release.add_permits(1);
        let final_report = later_dm_report(&j, &delivery, &review, "provider rejected").await;
        let observed = if already_matches {
            "Approved"
        } else {
            "Pending"
        };
        assert_row_report(&final_report, &j, observed, 3);
        assert!(
            final_report.contains("not confirmed as successful")
                && final_report.contains("observed state only")
        );
        {
            let ledger = j.ledger.lock().unwrap();
            assert_eq!(ledger.rejected, [j.script.selected.clone()]);
            assert_eq!(ledger.target, observed);
            assert_eq!(ledger.control, "Untouched");
            assert_eq!(
                ledger.writes,
                [(
                    j.script.selected.clone(),
                    json!({"fields":{"Status":"Approved"}})
                )]
            );
        }
        let binding = review.supervisor_write.as_ref().unwrap();
        wait_idle(&j, &binding.specialist_conversation).await;
        wait_idle(&j, &binding.source.channel.conversation_id).await;
        // A different current occurrence uses the saved permission, a fresh
        // exact DM decision, and a real successful exchange. The older failed
        // receipt must not contaminate its readback or owner-facing result.
        j.script.reject_write.store(false, Ordering::SeqCst);
        j.script.hold_readback.store(false, Ordering::SeqCst);
        let result = marked_assign(&j, &j.script.selection(), "ws7-after-rejection").await;
        let (next, code) = wait_review(&j).await;
        assert_ne!(next.id, review.id);
        let _ = body_text(owner(&j, &format!("yes write-{code}"), "ws7-after-rejection-yes").await)
            .await;
        let report = owner_text(
            &tokio::time::timeout(Duration::from_secs(25), result)
                .await
                .unwrap()
                .unwrap(),
        );
        assert_row_report(&report, &j, "Approved", 5);
        assert!(
            !report.contains("provider rejected")
                && !report.contains("not confirmed as successful")
        );
        let ledger = j.ledger.lock().unwrap();
        assert_eq!(ledger.writes.len(), 2);
        assert_eq!(ledger.rejected.len(), 1);
        assert_eq!(ledger.target, "Approved");
        assert_eq!(ledger.control, "Untouched");
        drop(ledger);
        remove_adapter(&j);
    }
}

#[tokio::test]
async fn supervisor_write_marked_wrong_event_kind_or_assignment_cannot_resume_or_read() {
    for fault in ["kind", "assignment", "invocation"] {
        let j = Box::pin(marked_journey(false)).await;
        let proposal = initial_gap(&j).await;
        let p = proposals::proposal(&proposal).unwrap();
        proposals::decide(
            &j.state,
            OWNER,
            &proposal.id,
            &choice(&proposal, "operation", &Uuid::new_v4().to_string()),
        )
        .await
        .unwrap();
        // The real web decision creates the durable source. Change one
        // negative fixture property, then consume it via the normal wake path.
        let field = match fault {
            "kind" => "pending_events.$[decision].kind",
            "assignment" => "pending_events.$[decision].report_to",
            _ => "pending_events.$[decision].invocation_id",
        };
        let mut set = doc! {};
        set.insert(
            field,
            if fault == "kind" {
                "message"
            } else {
                "unrelated-source"
            },
        );
        j.state
            .db
            .collection::<Document>("assistant_conversations")
            .update_one(doc! {"_id":&p.conversation_id}, doc! {"$set":set})
            .array_filters(vec![
                doc! {"decision.id":format!("execution-decision:{}",proposal.id)},
            ])
            .await
            .unwrap();
        crate::handlers::assistant_team::wake(&j.state, OWNER, &p.conversation_id).await;
        wait_idle(&j, &p.conversation_id).await;
        let stored = load(&j, &proposal.id).await;
        assert_eq!(
            proposals::proposal(&stored).unwrap().state,
            "queued",
            "{fault}"
        );
        assert_no_write(&j);
        assert_eq!(
            j.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {"supervisor_write":{"$ne":null}})
                .await
                .unwrap(),
            0
        );
        assert!(
            !j.observed
                .lock()
                .await
                .iter()
                .any(|e| e["stage"] == "retained_readback")
        );
        remove_adapter(&j);
    }
}

async fn wait_bound(j: &Journey) -> ApprovalRequest {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(row) = j
                .state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"user_id":OWNER,"supervisor_write.version":2})
                .await
                .unwrap()
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the marked independent requirement must remain visible")
}

#[tokio::test]
async fn supervisor_write_marked_retained_http_and_mcp_need_web_and_dm_then_specialist_readback() {
    for (proxy, failed_read) in [(false, false), (true, false), (false, true)] {
        let j = Box::pin(marked_journey(proxy)).await;
        j.script.repeat_after_resume.store(true, Ordering::SeqCst);
        j.script.readback_fault.store(failed_read, Ordering::SeqCst);
        let proposal = initial_gap(&j).await;
        let key = Uuid::new_v4().to_string();
        web_allow(&j, &proposal, "operation", &key).await;
        let (review, code) = wait_review(&j).await;
        let binding = review.supervisor_write.as_ref().unwrap();
        assert_eq!(binding.version, 2);
        let task = &proposals::proposal(&proposal).unwrap().task;
        assert!(
            !task
                .channel_source
                .as_ref()
                .unwrap()
                .audience_label
                .is_empty()
        );
        assert!(binding.source.channel.audience_label.is_empty());
        let mut display_only = task.clone();
        display_only
            .channel_source
            .as_mut()
            .unwrap()
            .audience_label
            .clear();
        assert!(crate::services::supervisor_write::same_task(
            task,
            &display_only
        ));
        display_only
            .channel_source
            .as_mut()
            .unwrap()
            .event_id
            .push_str("-different");
        assert!(!crate::services::supervisor_write::same_task(
            task,
            &display_only
        ));
        assert_eq!(
            binding.marked.as_ref().unwrap().operation_id,
            proposals::proposal(&proposal).unwrap().effect_id
        );
        assert_ne!(
            binding.specialist_turn,
            proposals::proposal(&proposal).unwrap().source_turn_id
        );
        assert_eq!(
            binding.marked.as_ref().unwrap().origin_turn,
            proposals::proposal(&proposal).unwrap().source_turn_id
        );
        assert_no_write(&j);
        reject_ordinary_web(&j, &review).await;
        reject_other_routes(&j, &review, &code).await;
        let delivery = cursor(&j).await;
        let dm = body_text(owner(&j, &format!("yes write-{code}"), "ws7-marked-dm").await).await;
        assert!(dm.contains("Write approval saved"));
        assert_eq!(
            j.state
                .db
                .collection::<Document>("notification_channels")
                .count_documents(doc! {"user_id":OWNER})
                .await
                .unwrap(),
            0,
            "the exact marked review must not materialize default settings after capture"
        );
        let terminal = wait_terminal(&j, &proposal.id).await;
        assert_eq!(
            proposals::proposal(&terminal).unwrap().state,
            "completed",
            "{}",
            review_diagnostics(&j).await
        );
        let marker = if failed_read {
            NO_CONFIRMATION.to_owned()
        } else {
            format!("provider-search-3:{}", j.script.selected)
        };
        let final_text = later_dm_report(&j, &delivery, &review, &marker).await;
        if failed_read {
            assert_eq!(final_text, NO_CONFIRMATION);
        } else {
            assert_row_report(&final_text, &j, "Approved", 3);
        }
        let observed = j.observed.lock().await;
        assert!(observed.iter().any(|e| e["stage"] == "retained_readback"));
        let readback = observed
            .iter()
            .find(|e| e["stage"] == "retained_readback")
            .unwrap();
        assert_eq!(
            readback["requested"],
            json!({"base":"bascnLedger","table":"tblLedger","record":j.script.selected})
        );
        assert_ne!(readback["requested"]["record"], j.script.control);
        assert!(observed.iter().any(|e| e["stage"] == "repeat_after_resume"));
        assert!(observed.iter().any(|e| {
            e["stage"] == "supervisor_event"
                && e["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|source| source["agent_id"].as_str().is_some())
        }));
        drop(observed);
        {
            let ledger = j.ledger.lock().unwrap();
            assert_eq!(
                ledger.writes,
                [(
                    j.script.selected.clone(),
                    json!({"fields":{"Status":"Approved"}})
                )]
            );
            assert_eq!(ledger.reads, ["Pending", "Pending", "Approved"]);
            assert_eq!(ledger.control, "Untouched");
        }
        // Both the channel event and typed web event are replayed through their
        // normal entry points. Neither can submit this occurrence again.
        let _ = body_text(owner(&j, &format!("yes write-{code}"), "ws7-marked-dm").await).await;
        web_allow(&j, &proposal, "operation", &key).await;
        let parent = &binding.source.channel.conversation_id;
        wait_idle(&j, parent).await;
        assert_eq!(j.ledger.lock().unwrap().writes.len(), 1);
        remove_adapter(&j);
    }
}

#[tokio::test]
async fn supervisor_write_marked_persistent_coverage_keeps_a_fresh_exact_dm_floor() {
    for proxy in [false, true] {
        let j = Box::pin(marked_journey(proxy)).await;
        let proposal = initial_gap(&j).await;
        web_allow(&j, &proposal, "persistent", &Uuid::new_v4().to_string()).await;
        let (review, code) = wait_review(&j).await;
        let delivery = cursor(&j).await;
        assert_no_write(&j);
        let _ =
            body_text(owner(&j, &format!("no write-{code}"), "ws7-marked-deny-first").await).await;
        assert_eq!(
            proposals::proposal(&wait_terminal(&j, &proposal.id).await)
                .unwrap()
                .state,
            "not_sent"
        );
        assert_no_write(&j);
        later_dm_report(&j, &delivery, &review, NO_CONFIRMATION).await;
        let binding = review.supervisor_write.as_ref().unwrap();
        wait_idle(&j, &binding.specialist_conversation).await;
        wait_idle(&j, &binding.source.channel.conversation_id).await;
        let result = marked_assign(&j, &j.script.selection(), "ws7-marked-covered").await;
        let (next, code) = wait_review(&j).await;
        assert_ne!(review.id, next.id);
        assert_eq!(
            j.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {"execution_proposal":{"$ne":null}})
                .await
                .unwrap(),
            1,
            "the actual saved persistent permission covers this new assignment"
        );
        assert_no_write(&j);
        reject_ordinary_web(&j, &next).await;
        let _ = body_text(owner(&j, &format!("yes write-{code}"), "ws7-marked-covered-yes").await)
            .await;
        let report = owner_text(
            &tokio::time::timeout(Duration::from_secs(25), result)
                .await
                .unwrap()
                .unwrap(),
        );
        assert_row_report(&report, &j, "Pending", 3);
        assert_row_report(&report, &j, "Approved", 4);
        let ledger = j.ledger.lock().unwrap();
        assert_eq!(
            ledger.writes,
            [(
                j.script.selected.clone(),
                json!({"fields":{"Status":"Approved"}})
            )]
        );
        assert_eq!(ledger.control, "Untouched");
        drop(ledger);
        remove_adapter(&j);
    }
}

#[tokio::test]
async fn supervisor_write_marked_unrenderable_effects_refuse_before_coverage_or_web_fallback() {
    for proxy in [false, true] {
        let j = Box::pin(marked_journey(proxy)).await;
        let proposal = initial_gap(&j).await;
        web_allow(&j, &proposal, "persistent", &Uuid::new_v4().to_string()).await;
        let (review, code) = wait_review(&j).await;
        let delivery = cursor(&j).await;
        let _ =
            body_text(owner(&j, &format!("no write-{code}"), "ws7-unsafe-first-deny").await).await;
        wait_terminal(&j, &proposal.id).await;
        later_dm_report(&j, &delivery, &review, NO_CONFIRMATION).await;
        let binding = review.supervisor_write.as_ref().unwrap();
        wait_idle(&j, &binding.specialist_conversation).await;
        wait_idle(&j, &binding.source.channel.conversation_id).await;
        let before = j
            .state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap();
        let faults: &[ReviewFault] = if proxy {
            &[
                ReviewFault::CallerCondition,
                ReviewFault::DuplicateMedia,
                ReviewFault::DefaultMedia,
                ReviewFault::GeneratedCorrelationOverride,
                ReviewFault::CallerCorrelation,
                ReviewFault::CustomCallerOverride,
                ReviewFault::UnsafeValue,
                ReviewFault::WrappedBody,
                ReviewFault::DuplicateField,
            ]
        } else {
            &[ReviewFault::DefaultMedia, ReviewFault::UnsafeValue]
        };
        for (index, fault) in faults.iter().enumerate() {
            use crate::models::default_request_header::DefaultRequestHeader;
            let defaults = match fault {
                ReviewFault::DefaultMedia => vec![("Content-Type", "text/plain", false)],
                ReviewFault::GeneratedCorrelationOverride => {
                    vec![("X-Request-Id", "configured-override", false)]
                }
                ReviewFault::CustomCallerOverride => vec![("X-Ledger-Mode", "configured", true)],
                _ => Vec::new(),
            };
            let stored = crate::services::default_header_service::encode(
                Some(&j.state.encryption_keys),
                "user_services",
                &binding.connection,
                OWNER,
                defaults
                    .into_iter()
                    .map(|(name, value, overridable)| DefaultRequestHeader {
                        name: name.into(),
                        value: value.to_owned().into(),
                        overridable,
                        sensitive: false,
                    })
                    .collect(),
            )
            .await
            .unwrap();
            j.state
                .db
                .collection::<Document>("user_services")
                .update_one(
                    doc! {"_id":&binding.connection,"user_id":OWNER},
                    doc! {"$set":{"default_request_headers":bson::to_bson(&stored).unwrap()}},
                )
                .await
                .unwrap();
            *j.script.review_fault.lock().unwrap() = Some(*fault);
            let observed_before = j.observed.lock().await.len();
            let result =
                marked_assign(&j, &j.script.selection(), &format!("ws7-unsafe-{index}")).await;
            let report = owner_text(
                &tokio::time::timeout(Duration::from_secs(25), result)
                    .await
                    .unwrap_or_else(|_| panic!("unsafe {fault:?} stalled instead of refusing"))
                    .unwrap(),
            );
            assert_eq!(report, NO_CONFIRMATION, "{fault:?}");
            let observations = j.observed.lock().await;
            let result = &observations
                .iter()
                .skip(observed_before)
                .find(|event| event["stage"] == "write")
                .expect("actual connected caller result")["result"];
            assert!(
                result
                    .to_string()
                    .contains("exact ledger-write review is unavailable"),
                "{fault:?}: {result}"
            );
            if proxy {
                assert_eq!(result["status"], 403, "{fault:?}");
            } else {
                assert!(tool_value(result).is_none(), "{fault:?}");
            }
            drop(observations);
            assert_no_write(&j);
            assert_eq!(
                j.state
                    .db
                    .collection::<Document>("approval_requests")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                before,
                "{fault:?} cannot downgrade to a proposal or ordinary approval"
            );
            wait_idle(&j, &binding.specialist_conversation).await;
            wait_idle(&j, &binding.source.channel.conversation_id).await;
        }
        remove_adapter(&j);
    }
}

#[tokio::test]
async fn supervisor_write_marked_delivery_refusal_and_uncertainty_never_fall_back_or_resume_twice()
{
    for uncertain in [false, true] {
        let j = Box::pin(marked_transport(false, "native")).await;
        let proposal = initial_gap(&j).await;
        if uncertain {
            Mock::given(method("POST"))
                .and(wiremock::matchers::path(
                    "/events/opaque-native-ref/side-messages",
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"delivery":{"outcome":"unknown"}})),
                )
                .with_priority(1)
                .mount(&j.chat)
                .await;
        } else {
            Mock::given(method("GET"))
                .and(wiremock::matchers::path(
                    "/events/opaque-native-ref/side-messages",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"available":false})))
                .with_priority(1)
                .mount(&j.chat)
                .await;
        }
        let key = Uuid::new_v4().to_string();
        web_allow(&j, &proposal, "operation", &key).await;
        let bound = wait_bound(&j).await;
        let before = bound.supervisor_write.as_ref().unwrap();
        assert!(!before.review_confirmed);
        assert!(
            crate::services::approval_service::fallback_supervisor_review(&j.state, &bound)
                .await
                .is_err()
        );
        reject_ordinary_web(&j, &bound).await;
        assert_eq!(
            proposals::proposal(&wait_terminal(&j, &proposal.id).await)
                .unwrap()
                .state,
            "not_sent"
        );
        let stored = load(&j, &bound.id).await;
        let after = stored.supervisor_write.as_ref().unwrap();
        assert_eq!(after.version, 2);
        assert_eq!(after.code_digest, before.code_digest);
        assert_eq!(
            after.source.channel.event_id,
            before.source.channel.event_id
        );
        assert!(after.dm_decision_receipt.is_none());
        assert_no_write(&j);
        wait_idle(&j, &proposals::proposal(&proposal).unwrap().conversation_id).await;
        web_allow(&j, &proposal, "operation", &key).await;
        wait_idle(&j, &proposals::proposal(&proposal).unwrap().conversation_id).await;
        assert_no_write(&j);
        assert_eq!(
            j.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {"supervisor_write.version":2})
                .await
                .unwrap(),
            1
        );
        remove_adapter(&j);
    }
}

#[tokio::test]
async fn supervisor_write_marked_expiry_revocation_and_legacy_status_do_not_release_the_write() {
    for control in ["expired", "revoked_permission", "legacy_status"] {
        let j = Box::pin(marked_journey(false)).await;
        let proposal = initial_gap(&j).await;
        web_allow(&j, &proposal, "operation", &Uuid::new_v4().to_string()).await;
        let (review, code) = wait_review(&j).await;
        assert_no_write(&j);
        if control == "expired" {
            j.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&review.id},
                doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))}}).await.unwrap();
        } else if control == "legacy_status" {
            j.state
                .db
                .collection::<Document>("approval_requests")
                .update_one(
                    doc! {"_id":&review.id},
                    doc! {"$set":{"status":"approved","decision_channel":"web"}},
                )
                .await
                .unwrap();
        } else {
            let approved = load(&j, &proposal.id).await;
            j.state.db.collection::<Document>("approval_grants").update_one(
                doc! {"_id":proposals::proposal(&approved).unwrap().grant_id.as_deref().unwrap()},
                doc! {"$set":{"revoked":true}}).await.unwrap();
            let _ =
                body_text(owner(&j, &format!("yes write-{code}"), "ws7-marked-revoked-yes").await)
                    .await;
        }
        assert_ne!(
            proposals::proposal(&wait_terminal(&j, &proposal.id).await)
                .unwrap()
                .state,
            "completed"
        );
        assert_no_write(&j);
        remove_adapter(&j);
    }
}

#[tokio::test]
async fn supervisor_write_marked_older_retained_payload_without_assignment_proof_refuses() {
    let j = Box::pin(marked_journey(true)).await;
    let proposal = initial_gap(&j).await;
    let p = proposals::proposal(&proposal).unwrap();
    // Synthesize the actual old encrypted v4 payload shape, retaining its
    // authenticated field context, occurrence and original web-only decision.
    let context = crate::crypto::record_field::RecordContext::new(
        "approval_requests",
        &proposal.id,
        OWNER,
        &["execution_proposal", "v4", "payload"],
    )
    .unwrap();
    assert_eq!(p.payload_context_version, Some(3));
    let raw = j
        .state
        .encryption_keys
        .decrypt_record(&p.payload.as_ref().unwrap().bytes, &context)
        .await
        .unwrap();
    let mut effect: Value = serde_json::from_slice(&raw).unwrap();
    assert!(
        effect
            .as_object_mut()
            .unwrap()
            .remove("supervisor")
            .is_some()
    );
    let legacy = j
        .state
        .encryption_keys
        .encrypt_record(&serde_json::to_vec(&effect).unwrap(), &context)
        .await
        .unwrap();
    j.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&proposal.id,"status":"pending"},
        doc! {"$set":{"execution_proposal.version":4,"execution_proposal.v4.payload":bson::Binary {
            subtype:bson::spec::BinarySubtype::Generic,bytes:legacy
        }}}).await.unwrap();
    let older = load(&j, &proposal.id).await;
    web_allow(&j, &older, "operation", &Uuid::new_v4().to_string()).await;
    assert_eq!(
        proposals::proposal(&wait_terminal(&j, &proposal.id).await)
            .unwrap()
            .state,
        "not_sent"
    );
    assert_no_write(&j);
    assert_eq!(
        j.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {"supervisor_write":{"$ne":null}})
            .await
            .unwrap(),
        0
    );
    remove_adapter(&j);
}

#[tokio::test]
async fn supervisor_write_marked_terminal_recovery_is_unverifiable_and_never_repeats() {
    let j = Box::pin(marked_journey(false)).await;
    j.script.hold_readback.store(true, Ordering::SeqCst);
    j.script.readback_fault.store(true, Ordering::SeqCst);
    let proposal = initial_gap(&j).await;
    web_allow(&j, &proposal, "operation", &Uuid::new_v4().to_string()).await;
    let (review, code) = wait_review(&j).await;
    let _ = body_text(owner(&j, &format!("yes write-{code}"), "ws7-recovery-yes").await).await;
    let completed = wait_terminal(&j, &proposal.id).await;
    let p = proposals::proposal(&completed).unwrap();
    assert!(
        p.payload.is_none(),
        "terminal recovery must work after payload purge"
    );
    tokio::time::timeout(Duration::from_secs(15), async {
        while !j
            .observed
            .lock()
            .await
            .iter()
            .any(|e| e["stage"] == "readback_started")
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let row = engine::get(&j.state.db, OWNER, &p.conversation_id)
        .await
        .unwrap();
    let egress = crate::handlers::assistant_nyxagent::http_egress(
        j.state.clone(),
        crate::test_utils::test_auth_user(OWNER),
    );
    let recovered = proposals::resume_turn_with_readback(&j.state, egress.as_ref(), &row)
        .await
        .unwrap()
        .unwrap();
    assert!(recovered.text.contains("unconfirmed"));
    assert!(recovered.specialist_readback.is_none());
    assert_eq!(j.ledger.lock().unwrap().writes.len(), 1);
    // A process lost after dispatch is recovered from its actual typed source,
    // without the purged body and without a second provider attempt.
    j.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&proposal.id},doc! {
        "$set":{"execution_proposal.v4.state":"dispatched",
            "execution_proposal.v4.claimed_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::minutes(3))}
    }).await.unwrap();
    for _ in 0..2 {
        let recovered = proposals::resume_turn_with_readback(&j.state, egress.as_ref(), &row)
            .await
            .unwrap()
            .unwrap();
        assert!(recovered.text.contains("unknown") && recovered.text.contains("cannot prove"));
        assert!(recovered.specialist_readback.is_none());
        assert_eq!(j.ledger.lock().unwrap().writes.len(), 1);
    }
    let delivery = cursor(&j).await;
    j.script.release.add_permits(1);
    let report = later_dm_report(&j, &delivery, &review, NO_CONFIRMATION).await;
    assert_eq!(report, NO_CONFIRMATION);
    wait_idle(&j, &p.conversation_id).await;
    let source = p.task.channel_source.as_ref().unwrap();
    j.state
        .db
        .collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&source.channel_id},
            doc! {"$set":{"owner_sender_ids":[]}},
        )
        .await
        .unwrap();
    assert!(
        proposals::resume_turn_with_readback(&j.state, egress.as_ref(), &row)
            .await
            .is_err()
    );
    let ledger = j.ledger.lock().unwrap();
    assert_eq!(ledger.writes.len(), 1);
    assert_eq!(ledger.control, "Untouched");
    drop(ledger);
    remove_adapter(&j);
}

#[tokio::test]
async fn supervisor_write_marked_coalesced_decisions_preserve_notices_and_do_not_choose_a_read_target()
 {
    for failed in [false, true] {
        let j = Box::pin(marked_journey(false)).await;
        let approved = initial_gap_stage(&j, 2).await;
        j.script.reject_write.store(failed, Ordering::SeqCst);
        let denied = j
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {
            "execution_proposal.version":5,"resource":{"$regex":format!("/{}$",j.script.control)}
        })
        .await
        .unwrap()
        .unwrap();
        // Commit both real web decisions before the ordinary wake so the engine
        // consumes one turn with mixed outcomes, as in the coalescing contract.
        proposals::decide(
            &j.state,
            OWNER,
            &approved.id,
            &choice(&approved, "operation", &Uuid::new_v4().to_string()),
        )
        .await
        .unwrap();
        let mut rejection = choice(&denied, "operation", &Uuid::new_v4().to_string());
        rejection.approved = false;
        proposals::decide(&j.state, OWNER, &denied.id, &rejection)
            .await
            .unwrap();
        let conversation = proposals::proposal(&approved)
            .unwrap()
            .conversation_id
            .clone();
        crate::handlers::assistant_team::wake(&j.state, OWNER, &conversation).await;
        let (review, code) = wait_review(&j).await;
        assert_no_write(&j);
        assert!(
            review
                .resource
                .as_ref()
                .unwrap()
                .ends_with(&j.script.selected)
        );
        let delivery = cursor(&j).await;
        let _ = body_text(owner(&j, &format!("yes write-{code}"), "ws7-coalesced-dm").await).await;
        let report = later_dm_report(&j, &delivery, &review, &approved.id).await;
        assert!(
            report.contains(&denied.id)
                && report.contains("denied")
                && report.contains("unconfirmed")
        );
        assert_eq!(report.contains("provider rejected"), failed);
        assert!(
            !j.observed
                .lock()
                .await
                .iter()
                .any(|event| event["stage"] == "retained_readback")
        );
        let ledger = j.ledger.lock().unwrap();
        assert_eq!(ledger.writes.len(), 1);
        assert_eq!(ledger.writes[0].0, j.script.selected);
        assert_eq!(ledger.target, if failed { "Pending" } else { "Approved" });
        assert_eq!(ledger.control, "Untouched");
        drop(ledger);
        remove_adapter(&j);
    }
}
