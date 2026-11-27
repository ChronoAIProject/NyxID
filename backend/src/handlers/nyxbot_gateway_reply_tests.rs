use super::*;
use crate::services::channel_turn_preparation as prep;

async fn pending(name: &str, status_code: u16, code: &str) -> Fixture {
    let f = fixture(name).await;
    settled(&f, false).await;
    patch(&f, doc! {"delivery.state":"pending"}).await;
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(ResponseTemplate::new(status_code).set_body_json(json!({"detail":code})))
        .with_priority(1)
        .mount(&f.gateway)
        .await;
    late_delivery::process(&f.state, &f.event.id).await;
    f
}
async fn raw(f: &Fixture) -> bson::Document {
    f.state
        .db
        .collection(EVENTS)
        .find_one(doc! {"_id":&f.event.id})
        .await
        .unwrap()
        .unwrap()
}
async fn due(f: &Fixture) {
    patch(
        f,
        doc! {"delivery.receipt_due_at":bson::DateTime::from_millis(0)},
    )
    .await;
}
async fn receipt(f: &Fixture) {
    f.gateway.reset().await;
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
        )
        .mount(&f.gateway)
        .await;
}
async fn outcomes(f: &Fixture) -> Vec<bson::Document> {
    f.state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":"nyxbot_channel_answer_delivery"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn answer_health(f: &Fixture) -> Value {
    let bot = crate::services::channel_bot_service::get_bot(&f.state.db, &f.channel.channel_bot_id)
        .await
        .unwrap();
    let health = Box::pin(crate::handlers::channel_bots::health_loaded(
        &f.state, &bot, true, false,
    ))
    .await
    .unwrap();
    serde_json::to_value(health).unwrap()
}

#[tokio::test]
async fn gateway_reply_pending_survivors_use_identical_request_and_one_receipt() {
    let f = pending(
        "gateway_reply_pending_survivors",
        409,
        "routed_reply_in_progress",
    )
    .await;
    assert_eq!(status(&f).await, "unknown");
    let health = answer_health(&f).await;
    assert_eq!(health["answer_delivery"]["state"], "unconfirmed");
    assert_eq!(
        health["answer_delivery"]["reason"],
        "gateway_receipt_pending"
    );
    assert_eq!(health["recommended_action"], "none");
    patch(
        &f,
        doc! {"created_at": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::minutes(11))},
    )
    .await;
    let stalled = answer_health(&f).await;
    assert_eq!(stalled["answer_delivery"]["state"], "unconfirmed");
    assert_eq!(stalled["answer_delivery"]["reason"], "delivery_stalled");
    assert_eq!(stalled["recommended_action"], "verify");
    let first = sends(&f).await.pop().unwrap();
    let before = raw(&f).await;
    assert!(!before.to_string().contains("Useful answer"));
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(EVENTS)
            .count_documents(prep::due())
            .await
            .unwrap(),
        0
    );
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1, "not due yet");
    receipt(&f).await;
    due(&f).await;
    let (a, b) = tokio::join!(
        late_delivery::sweep(&f.state),
        late_delivery::sweep(&f.state)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(status(&f).await, "sent");
    let health = answer_health(&f).await;
    assert_eq!(health["answer_delivery"]["state"], "confirmed");
    assert_eq!(health["recommended_action"], "none");
    let requests = sends(&f).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), first.url.path());
    assert_eq!(requests[0].body, first.body);
    assert_eq!(
        requests[0].headers.get("idempotency-key"),
        first.headers.get("idempotency-key")
    );
    assert_eq!(outcomes(&f).await.len(), 2);
    assert!(
        !raw(&f)
            .await
            .get_document("delivery")
            .unwrap()
            .contains_key("receipt_due_at")
    );
}

#[tokio::test]
async fn gateway_reply_exact_503_retries_without_claiming_admission() {
    let f = pending("gateway_reply_transient", 503, "routed_reply_unavailable").await;
    let row = raw(&f).await;
    let wait = row
        .get_document("delivery")
        .unwrap()
        .get_document("preparation")
        .unwrap()
        .get_document("gateway_receipt")
        .unwrap();
    assert!(wait.get("admitted_at").unwrap().as_null().is_some());
    assert!(wait.get_bool("pending").unwrap());
    assert!(outcomes(&f).await.is_empty());
    let bot = f
        .state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.channel.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bot.get_document("diagnostics")
            .unwrap()
            .get_document("last_outbound_error")
            .unwrap()
            .get_str("reason")
            .unwrap(),
        "gateway_reply_unavailable"
    );
    receipt(&f).await;
    due(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(status(&f).await, "sent");
    assert_eq!(outcomes(&f).await.len(), 1);
}

#[tokio::test]
async fn gateway_reply_expiry_stays_unconfirmed_and_health_reports_the_gateway() {
    let f = pending("gateway_reply_expiry", 409, "routed_reply_in_progress").await;
    patch(
        &f,
        doc! {"delivery.preparation.gateway_receipt.deadline":bson::DateTime::from_millis(0)},
    )
    .await;
    due(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(status(&f).await, "unknown");
    assert_eq!(outcomes(&f).await.len(), 2);
    let bot = f
        .state
        .db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&f.channel.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    let health = answer_health(&f).await;
    assert_eq!(health["answer_delivery"]["state"], "unconfirmed");
    assert_eq!(
        health["answer_delivery"]["reason"],
        "platform_delivery_unconfirmed"
    );
    assert_ne!(health["recommended_action"], "none");
    assert!(
        health["recommended_action_text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("check the chat")
    );
    let advice = crate::services::channel_bot_health::recommend(
        &crate::services::channel_bot_health::HealthFacts::for_bot(&bot, Utc::now(), true),
    );
    assert_eq!(advice.action, "gateway");
    assert!(advice.text.contains("gateway accepted"));
    assert!(advice.text.contains("Check the chat"));
    let serialized = serde_json::to_value(&bot.diagnostics).unwrap();
    assert_eq!(
        serialized["last_outbound_error"]["reason"],
        "gateway_reply_unconfirmed"
    );
}

#[tokio::test]
async fn gateway_reply_refusals_and_other_uncertainty_never_retry() {
    for (status_code, code, expected) in [
        (400, "routed_reply_invalid", "refused"),
        (403, "routed_reply_revoked", "refused"),
        (410, "event_ref_expired", "refused"),
        (409, "reply_idempotency_conflict", "refused"),
        (429, "reply_operation_limit", "refused"),
    ] {
        let f = pending(
            &format!("gateway_reply_refusal_{status_code}_{expected}"),
            status_code,
            code,
        )
        .await;
        assert_eq!(status(&f).await, expected);
        late_delivery::sweep(&f.state).await.unwrap();
        assert_eq!(sends(&f).await.len(), 1);
        let audits = outcomes(&f).await;
        assert_eq!(audits.len(), 1);
        let audit = audits[0].to_string();
        assert!(audit.contains(if expected == "refused" {
            "rejected"
        } else {
            "unknown"
        }));
        assert!(!audit.contains("Useful answer"));
        assert!(!audit.contains("cmaeg1.rt"));
    }
}

async fn bot_diagnostics(f: &Fixture) -> bson::Bson {
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.channel.channel_bot_id})
        .await
        .unwrap()
        .unwrap()
        .get("diagnostics")
        .cloned()
        .unwrap_or(bson::Bson::Null)
}

fn channel_milestones(rows: &[Value]) -> Vec<&str> {
    rows.iter()
        .filter(|r| r["stage"] == "channel" && r["phase"] == "milestone")
        .map(|r| r["outcome"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn gateway_local_refusals_keep_local_truth_and_received_403_is_gateway_rejection() {
    for reason in [
        "no_agent_key",
        "target_expired",
        "target_expiry_missing",
        "legacy_event_expired",
        "authority_changed",
        "received_403",
    ] {
        let f = fixture(&format!("gateway_local_refusal_{reason}")).await;
        settled(&f, false).await;
        let target = event_record::seal(
            &f.state.encryption_keys,
            &f.event.id,
            &f.event.user_id,
            &[event_record::TARGET, &f.event.channel_id],
            b"cmaeg1.rt.original",
        )
        .await
        .unwrap();
        patch(&f, doc! {
            "delivery.state":"pending",
            "delivery.target_ciphertext":bson::Binary {subtype:bson::spec::BinarySubtype::Generic, bytes:target},
            "delivery.target_context_version": event_record::version(&f.state.encryption_keys).map(i32::from),
            "delivery.target_expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1)),
            "dispatch_identity":bson::to_bson(&dispatch_identity::admission(&f.channel.id,"original-post")).unwrap(),
        }).await;
        match reason {
            "no_agent_key" => {
                f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id":&f.channel.id},
                    doc! {
                        "$set":{"agent_key_ciphertext":bson::Bson::Null},
                        "$unset":{channel_key::VERSION_PATH:""},
                    }).await.unwrap();
            }
            "target_expired" => patch(&f, doc! {"delivery.target_expires_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::minutes(1))}).await,
            "target_expiry_missing" => patch(&f, doc! {"delivery.target_expires_at":bson::Bson::Null}).await,
            "legacy_event_expired" => patch(&f, doc! {
                "delivery.target_ciphertext":bson::Bson::Null,
                "delivery.target_context_version":bson::Bson::Null,
                "delivery.target_expires_at":bson::Bson::Null,
                "created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::minutes(30)),
            }).await,
            "received_403" => {
                Mock::given(method("POST")).and(path("/reply-targets/cmaeg1.rt.original/messages"))
                    .respond_with(ResponseTemplate::new(403).set_body_json(json!({"detail":"routed_reply_revoked"})))
                    .with_priority(1).mount(&f.gateway).await;
            }
            _ => {},
        }
        let before = bot_diagnostics(&f).await;
        let (_, trace) = crate::turn_trace::capture(Box::pin(async {
            if reason == "authority_changed" {
                crate::services::durable_wake_tests::resume_after(
                    "late_text_prepared",
                    late_delivery::process(&f.state, &f.event.id),
                    async {
                        f.state
                            .db
                            .collection::<bson::Document>("users")
                            .update_one(doc! {"_id":OWNER}, doc! {"$set":{"is_active":false}})
                            .await
                            .unwrap();
                    },
                )
                .await;
                f.state
                    .db
                    .collection::<bson::Document>("users")
                    .update_one(doc! {"_id":OWNER}, doc! {"$set":{"is_active":true}})
                    .await
                    .unwrap();
            } else {
                late_delivery::process(&f.state, &f.event.id).await;
            }
        }))
        .await;
        assert_eq!(status(&f).await, "refused", "{reason}");
        assert_eq!(
            channel_milestones(&trace),
            ["rejected"],
            "{reason}: {trace:?}"
        );
        let row = raw(&f).await;
        let delivery = row.get_document("delivery").unwrap();
        let remote = reason == "received_403";
        let health = answer_health(&f).await;
        assert_eq!(health["answer_delivery"]["state"], "rejected", "{reason}");
        let expected_reason = if remote {
            "gateway_reply_rejected"
        } else {
            "answer_delivery_refused"
        };
        assert_eq!(
            health["answer_delivery"]["reason"], expected_reason,
            "{reason}"
        );
        assert_ne!(health["recommended_action"], "none", "{reason}");
        assert_eq!(
            delivery.get_bool("gateway_attempted").unwrap_or(false),
            remote,
            "{reason}"
        );
        assert_eq!(sends(&f).await.len(), usize::from(remote), "{reason}");
        if remote {
            let audits = outcomes(&f).await;
            assert_eq!(audits.len(), 1);
            let audit = audits[0].get_document("event_data").unwrap();
            assert_eq!(audit.get_str("outcome"), Ok("rejected"));
            assert_eq!(audit.get_str("http_status_class"), Ok("4xx"));
            assert_eq!(audit.get_str("gateway_code"), Ok("routed_reply_revoked"));
            assert_eq!(
                delivery
                    .get_document("gateway_observation")
                    .unwrap()
                    .get_i32("status"),
                Ok(403)
            );
            assert!(
                bot_diagnostics(&f)
                    .await
                    .to_string()
                    .contains("gateway_reply_rejected")
            );
        } else {
            assert!(
                f.gateway.received_requests().await.unwrap().is_empty(),
                "{reason}"
            );
            assert!(outcomes(&f).await.is_empty(), "{reason}");
            assert_eq!(bot_diagnostics(&f).await, before, "{reason}");
            assert!(!delivery.contains_key("gateway_observation"), "{reason}");
            assert!(!delivery.contains_key("gateway_reason"), "{reason}");
        }
        let dispatch_identity::Lookup::Found(found) = dispatch_identity::lookup(
            &f.state.db,
            &f.state.encryption_keys,
            &f.channel,
            "original-post",
            None,
            None,
            None,
        )
        .await
        .unwrap() else {
            panic!("admission alias missing: {reason}")
        };
        let found = serde_json::to_value(found).unwrap();
        assert_eq!(
            found["provider_delivery"],
            if remote { "unconfirmed" } else { "none" },
            "{reason}: {found}"
        );
        late_delivery::sweep(&f.state).await.unwrap();
        assert_eq!(sends(&f).await.len(), usize::from(remote));
    }
}

#[tokio::test]
async fn gateway_receipt_milestones_and_terminal_audit_keep_frozen_image_counts() {
    for disposition in [
        "confirmed",
        "rejected",
        "unknown",
        "pending_confirmed",
        "pending_expired",
        "unavailable_admitted",
    ] {
        let f = fixture(&format!("gateway_diagnostics_{disposition}")).await;
        settled(&f, false).await;
        patch(&f, doc! {"delivery.state":"pending"}).await;
        let message = f
            .state
            .db
            .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
            .find_one(doc! {"turn_id":&f.event.turn_id})
            .await
            .unwrap()
            .unwrap();
        prep::acquire(&f.state.db, &f.event.id, &message.id, "preparer")
            .await
            .unwrap()
            .unwrap();
        let notice = crate::models::channel_text_delivery::TextDeliveryNotice {
            attempted: 6,
            sent: 1,
            failed: 2,
            unknown: 3,
            skipped: 4,
            unsupported: false,
            app_visible: true,
            automation: false,
            omitted: 0,
        };
        patch(
            &f,
            doc! {"delivery.preparation.notice":bson::to_bson(&notice).unwrap(),
            "delivery.preparation.lease_until":bson::DateTime::from_millis(0)},
        )
        .await;
        let (status, body) = match disposition {
            "confirmed" => (200, json!({"delivery":{"outcome":"confirmed"}})),
            "rejected" => (403, json!({"detail":"routed_reply_revoked"})),
            "unknown" => (200, json!({"delivery":{"outcome":"unknown"}})),
            "unavailable_admitted" => (503, json!({"detail":"routed_reply_unavailable"})),
            _ => (409, json!({"detail":"routed_reply_in_progress"})),
        };
        Mock::given(method("POST"))
            .and(path("/reply-targets/cmaeg1.rt.original/messages"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .with_priority(1)
            .mount(&f.gateway)
            .await;
        let (_, trace) =
            crate::turn_trace::capture(Box::pin(late_delivery::process(&f.state, &f.event.id)))
                .await;
        let pending = matches!(
            disposition,
            "pending_confirmed" | "pending_expired" | "unavailable_admitted"
        );
        assert_eq!(
            channel_milestones(&trace),
            match disposition {
                "confirmed" => vec!["confirmed"],
                "rejected" => vec!["rejected"],
                "unavailable_admitted" => vec![],
                _ => vec!["unknown"],
            },
            "{disposition}: {trace:?}"
        );
        if disposition == "unavailable_admitted" {
            f.gateway.reset().await;
            Mock::given(method("POST"))
                .and(path("/reply-targets/cmaeg1.rt.original/messages"))
                .respond_with(
                    ResponseTemplate::new(409)
                        .set_body_json(json!({"detail":"routed_reply_in_progress"})),
                )
                .mount(&f.gateway)
                .await;
            due(&f).await;
            let (result, trace) =
                crate::turn_trace::capture(Box::pin(late_delivery::sweep(&f.state))).await;
            result.unwrap();
            assert_eq!(channel_milestones(&trace), ["unknown"]);
        }
        if pending {
            assert_eq!(outcomes(&f).await.len(), 1);
            assert!(
                !outcomes(&f).await[0]
                    .get_document("event_data")
                    .unwrap()
                    .contains_key("images_attempted")
            );
            if disposition == "pending_expired" {
                patch(&f,doc! {"delivery.preparation.gateway_receipt.deadline":bson::DateTime::from_millis(0)}).await;
            } else {
                receipt(&f).await;
            }
            due(&f).await;
            let (result, trace) =
                crate::turn_trace::capture(Box::pin(late_delivery::sweep(&f.state))).await;
            result.unwrap();
            assert_eq!(
                channel_milestones(&trace),
                [if disposition == "pending_expired" {
                    "unknown"
                } else {
                    "confirmed"
                }]
            );
        }
        let audits = outcomes(&f).await;
        assert_eq!(audits.len(), if pending { 2 } else { 1 });
        let terminal = audits
            .iter()
            .map(|row| row.get_document("event_data").unwrap())
            .find(|row| row.get_str("outcome") != Ok("receipt_pending"))
            .unwrap();
        for (field, expected) in [
            ("images_attempted", 6),
            ("images_sent", 1),
            ("images_failed", 2),
            ("images_unknown", 3),
            ("images_skipped", 4),
        ] {
            assert_eq!(
                terminal
                    .get_i64(field)
                    .or_else(|_| terminal.get_i32(field).map(i64::from))
                    .unwrap(),
                expected,
                "{disposition}: {field}"
            );
        }
        let (result, trace) =
            crate::turn_trace::capture(Box::pin(late_delivery::sweep(&f.state))).await;
        result.unwrap();
        assert!(channel_milestones(&trace).is_empty());
        assert_eq!(outcomes(&f).await.len(), audits.len());
    }
}

#[tokio::test]
async fn gateway_reply_lost_poll_claim_is_fenced_and_source_change_never_sends() {
    let f = pending("gateway_reply_lost_claim", 409, "routed_reply_in_progress").await;
    due(&f).await;
    prep::gateway::acquire(&f.state.db, &f.event.id, "dead-worker")
        .await
        .unwrap()
        .unwrap();
    assert!(
        prep::gateway::acquire(&f.state.db, &f.event.id, "second-worker")
            .await
            .unwrap()
            .is_none()
    );
    due(&f).await;
    let winner = prep::gateway::acquire(&f.state.db, &f.event.id, "survivor")
        .await
        .unwrap()
        .unwrap();
    assert!(
        prep::gateway::before_effect(&f.state.db, &f.event.id, "dead-worker")
            .await
            .is_err()
    );
    assert!(
        !prep::gateway::record(
            &f.state.db,
            &f.event.id,
            "dead-worker",
            &f.channel.channel_bot_id,
            &prep::gateway::unknown(Default::default()),
            winner.gateway_receipt.as_ref(),
            winner.gateway_receipt.as_ref().unwrap().deadline,
            "test"
        )
        .await
        .unwrap()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"turn_id":&f.event.turn_id},
            doc! {"$set":{"text":"changed after admission"}},
        )
        .await
        .unwrap();
    due(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert!(
        !raw(&f)
            .await
            .get_document("delivery")
            .unwrap()
            .contains_key("receipt_due_at")
    );
}

#[tokio::test]
async fn gateway_reply_actual_preparation_and_replay_cas_both_orders() {
    let f = fixture("gateway_reply_cas").await;
    for replay_first in [true, false] {
        patch(&f, doc! {"delivery.state":"pending"}).await;
        prep::acquire(&f.state.db, &f.event.id, "message", "claim")
            .await
            .unwrap()
            .unwrap();
        if replay_first {
            assert!(
                relay_responses::claim_event_replay(&f.state.db, &f.channel, &f.event.id)
                    .await
                    .unwrap()
            );
            assert!(
                prep::before_effect(&f.state.db, &f.event.id, "claim", 0)
                    .await
                    .is_err()
            );
        } else {
            // plan supplies the actual cursor used at the effect barrier.
            patch(&f, doc! {"delivery.preparation.next":0}).await;
            prep::before_effect(&f.state.db, &f.event.id, "claim", 0)
                .await
                .unwrap();
            assert!(
                !relay_responses::claim_event_replay(&f.state.db, &f.channel, &f.event.id)
                    .await
                    .unwrap()
            );
        }
    }
    for _ in 0..12 {
        patch(
            &f,
            doc! {"delivery.state":"pending","delivery.preparation.next":0},
        )
        .await;
        prep::acquire(&f.state.db, &f.event.id, "message", "race")
            .await
            .unwrap()
            .unwrap();
        let (replay, send) = tokio::join!(
            relay_responses::claim_event_replay(&f.state.db, &f.channel, &f.event.id),
            prep::before_effect(&f.state.db, &f.event.id, "race", 0)
        );
        assert_ne!(replay.unwrap(), send.is_ok());
    }
}

async fn snapshot(
    db: &mongodb::Database,
) -> std::collections::BTreeMap<String, Vec<bson::Document>> {
    let mut result = std::collections::BTreeMap::new();
    for name in db.list_collection_names().await.unwrap() {
        if matches!(
            name.as_str(),
            "security_replicas" | "security_rollouts" | "system.profile"
        ) {
            continue;
        }
        let rows = db
            .collection::<bson::Document>(&name)
            .find(doc! {})
            .sort(doc! {"_id":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        result.insert(name, rows);
    }
    result
}
async fn lookup(f: &Fixture, key: &str, at: Option<i64>) -> Response {
    let mut headers = bearer(&f.key);
    headers.insert("idempotency-key", key.parse().unwrap());
    dispatches::get(
        State(f.state.clone()),
        headers,
        axum::extract::Query(dispatches::Query {
            dispatched_at: at.map(|v| v.to_string()),
        }),
    )
    .await
}

#[tokio::test]
async fn gateway_dispatch_group_aliases_lookup_states_and_read_only_snapshot() {
    let mut f = fixture("gateway_dispatch_group_aliases").await;
    crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
    let (db, commands) = crate::services::durable_wake_tests::monitored(&f.state.db).await;
    f.state.db = db;
    patch(&f,doc! {"status":"queued","dispatch_identity.version":1,"dispatch_identity.keys":[dispatch_identity::dispatch_key(&f.channel.id,"original-post")]}).await;
    for n in 1..32 {
        assert!(
            dispatch_identity::alias(
                &f.state.db,
                &f.event.id,
                &f.channel.id,
                &dispatch_identity::dispatch_key(&f.channel.id, &format!("alias-{n}"))
            )
            .await
            .unwrap()
        );
    }
    assert!(
        !dispatch_identity::alias(&f.state.db, &f.event.id, &f.channel.id, "overflow")
            .await
            .unwrap()
    );
    assert!(
        dispatch_identity::alias(
            &f.state.db,
            &f.event.id,
            &f.channel.id,
            &dispatch_identity::dispatch_key(&f.channel.id, "alias-31")
        )
        .await
        .unwrap()
    );
    for (delivery, execution, expected, provider) in [
        ("pending", "queued", "in_progress", None),
        ("pending", "retryable", "in_progress", None),
        ("pending", "completed", "finished", Some("none")),
        ("sending", "failed", "finished", Some("started")),
        ("sent", "completed", "finished", Some("confirmed")),
        ("unknown", "completed", "finished", Some("unconfirmed")),
    ] {
        patch(&f, doc! {"delivery.state":delivery,"status":execution}).await;
        let before = snapshot(&f.state.db).await;
        crate::services::durable_wake_tests::take(&commands);
        let response = lookup(&f, "alias-31", Some(Utc::now().timestamp())).await;
        let observed = crate::services::durable_wake_tests::take(&commands);
        let response_status = response.status();
        let response_body = body_text(response).await;
        assert_eq!(response_status, StatusCode::OK, "{response_body}");
        assert!(
            observed.keys().all(|key| !key.starts_with("insert:")
                && !key.starts_with("update:")
                && !key.starts_with("findAndModify:")
                && !key.starts_with("delete:")),
            "{observed:?}"
        );
        let body: Value = serde_json::from_str(&response_body).unwrap();
        assert_eq!(body["state"], expected);
        if let Some(provider) = provider {
            assert_eq!(body["provider_delivery"], provider);
            assert_eq!(body["stored_response"], false);
        }
        assert_eq!(snapshot(&f.state.db).await, before);
    }
    let old = lookup(&f, "missing", None).await;
    assert_eq!(old.status(), StatusCode::BAD_REQUEST);
    assert!(
        body_text(old)
            .await
            .contains("dispatch_identity_unavailable")
    );
    let missing = lookup(&f, "missing", Some(Utc::now().timestamp() + 1)).await;
    assert!(body_text(missing).await.contains("not_received"));
    let expired = lookup(
        &f,
        "missing",
        Some((Utc::now() - ChronoDuration::days(8)).timestamp()),
    )
    .await;
    assert!(body_text(expired).await.contains("expired"));
}

#[tokio::test]
async fn gateway_dispatch_wrong_key_inactive_and_readiness_refusals() {
    let f = fixture("gateway_dispatch_refusals").await;
    crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
    let mut headers = bearer("invalid");
    headers.insert("idempotency-key", "one".parse().unwrap());
    assert_eq!(
        dispatches::get(
            State(f.state.clone()),
            headers,
            axum::extract::Query(dispatches::Query {
                dispatched_at: None
            })
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    f.state
        .db
        .collection::<bson::Document>("security_rollouts")
        .delete_many(doc! {"capability":dispatch_identity::READINESS})
        .await
        .unwrap();
    let response = lookup(&f, "one", None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body_text(response)
            .await
            .contains("response_delivery_not_ready")
    );
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"status":"inactive"}},
        )
        .await
        .unwrap();
    assert_eq!(
        lookup(&f, "one", None).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn gateway_receipt_idle_query_uses_empty_index_bounds() {
    let f = fixture("gateway_receipt_idle_index").await;
    let explain = f
        .state
        .db
        .run_command(doc! {"explain":{
        "find":EVENTS,"filter":prep::gateway::due(Utc::now()),"sort":{"delivery.receipt_due_at":1},
        "limit":32,"hint":prep::gateway::DUE_INDEX},"verbosity":"executionStats"})
        .await
        .unwrap();
    let stats = explain.get_document("executionStats").unwrap();
    assert_eq!(stats.get_i32("nReturned").unwrap(), 0);
    assert_eq!(stats.get_i32("totalDocsExamined").unwrap(), 0);
    assert!(explain.to_string().contains(prep::gateway::DUE_INDEX));
}

#[tokio::test]
async fn gateway_reply_stored_get_events_and_repeated_post_are_empty_completed_contract() {
    use crate::{
        models::nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse},
        services::nyxbot_response_recovery as recovery,
    };
    let mut f = fixture("gateway_reply_empty_recovery").await;
    crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
    f.channel.binding_id = Some("test-binding".into());
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    let key = "stored-post";
    let id = dispatch_identity::relay_response(&f.channel.id, key);
    let request = json!({"conversation":PARTITION,"input":[],
        "metadata":{"cmaeg.channel":"channel","cmaeg.event":"event","cmaeg.partition":"partition","cmaeg.actor":"actor"},
        "event_context":{"conversation_id":PARTITION,"activity":{"event_id":"event"}}});
    let mut payload = recovery::Payload::new(
        &id,
        "nyxbot-fixture",
        recovery::metadata(&request["metadata"]).unwrap(),
    );
    payload.finish(Some("Useful answer"), true);
    let mut stored = GatewayResponse {
        id: Uuid::new_v4().to_string(),
        response_id: id.clone(),
        user_id: OWNER.into(),
        channel_id: f.channel.id.clone(),
        binding_id: f.channel.binding_id.clone().unwrap(),
        partition: PARTITION.into(),
        event_key: f.event.id.clone(),
        request_hash: sha256_hex(request.to_string()),
        conversation_id: f.event.conversation_id.clone(),
        terminal: true,
        answer_delivery: None,
        revision: 1,
        payload_context_version: None,
        payload_ciphertext: Vec::new(),
        created_at: Utc::now(),
        expires_at: Utc::now() + ChronoDuration::hours(1),
    };
    recovery::storage::install(&f.state.encryption_keys, &mut stored, &payload)
        .await
        .unwrap();
    assert_eq!(stored.payload_context_version, Some(3));
    f.state
        .db
        .collection::<GatewayResponse>(RESPONSES)
        .insert_one(stored)
        .await
        .unwrap();
    for state in ["sending", "sent", "unknown", "refused"] {
        patch(&f, doc! {"delivery.state":state,"status":"failed"}).await;
        let response =
            native_responses::snapshot(State(f.state.clone()), Path(id.clone()), bearer(&f.key))
                .await;
        assert_eq!(response.status(), StatusCode::OK);
        let snapshot: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert_eq!(snapshot["status"], "completed");
        assert_eq!(snapshot["output"], json!([]));
        assert_eq!(snapshot["id"], id);
        assert_eq!(snapshot["metadata"], request["metadata"]);
        for response in [
            native_responses::events(
                State(f.state.clone()),
                Path(id.clone()),
                axum::extract::Query(native_responses::Cursor {
                    starting_after: None,
                }),
                bearer(&f.key),
            )
            .await,
            respond(&f.state, &f.key, &request, key).await,
        ] {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get("x-cma-response-id").unwrap(),
                id.as_str()
            );
            let text = body_text(response).await;
            let frames: Vec<Value> = text
                .lines()
                .filter_map(|l| l.strip_prefix("data: "))
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            // CMA develop 92b66eb: frozen acceptance + recovery/stream decode
            // require identity, queued created, contiguous cursors and lifecycle status.
            assert_eq!(frames.len(), 3);
            for (index, frame) in frames.iter().enumerate() {
                assert_eq!(frame["sequence_number"], index);
                assert_eq!(frame["response"]["id"], id);
                assert_eq!(frame["response"]["object"], "response");
                assert_eq!(frame["response"]["conversation"]["id"], "nyxbot-fixture");
                assert_eq!(frame["response"]["metadata"], request["metadata"]);
                assert_eq!(frame["response"]["output"], json!([]));
            }
            assert_eq!(frames[0]["type"], "response.created");
            assert_eq!(frames[0]["response"]["status"], "queued");
            assert_eq!(frames[1]["type"], "response.in_progress");
            assert_eq!(frames[2]["type"], "response.completed");
            assert_eq!(frames[2]["response"]["status"], "completed");
            assert!(!text.contains("Useful answer"));
            assert!(!text.contains("response.failed"));
            for index in 0..3 {
                assert!(text.contains(&format!("id: {index}\n")));
            }
        }
        let resumed = native_responses::events(
            State(f.state.clone()),
            Path(id.clone()),
            axum::extract::Query(native_responses::Cursor {
                starting_after: Some("1".into()),
            }),
            bearer(&f.key),
        )
        .await;
        let text = body_text(resumed).await;
        assert!(text.contains("id: 2\n"));
        assert!(!text.contains("response.created"));
    }
    assert!(sends(&f).await.is_empty());
}

#[tokio::test]
async fn gateway_reply_crash_after_pending_restarts_without_target_mint() {
    let f = fixture("gateway_reply_pending_crash").await;
    settled(&f, false).await;
    patch(&f, doc! {"delivery.state":"pending"}).await;
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(json!({"detail":"routed_reply_in_progress"})),
        )
        .with_priority(1)
        .mount(&f.gateway)
        .await;
    crate::services::durable_wake_tests::drop_after(
        "late_text_receipt_committed",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    assert_eq!(status(&f).await, "unknown");
    receipt(&f).await;
    due(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(status(&f).await, "sent");
    assert!(
        f.gateway
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path().ends_with("/messages"))
    );
}

#[tokio::test]
async fn gateway_dispatch_rollback_aliases_never_prove_nonacceptance() {
    let f = fixture("gateway_dispatch_rollback").await;
    crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
    // Old admission, new duplicate alias: the missing original key is still ambiguous.
    assert!(
        dispatch_identity::alias(
            &f.state.db,
            &f.event.id,
            &f.channel.id,
            &dispatch_identity::dispatch_key(&f.channel.id, "new-key")
        )
        .await
        .unwrap()
    );
    let found = lookup(&f, "new-key", None).await;
    assert_eq!(found.status(), StatusCode::OK);
    let refused = lookup(&f, "lost-old-key", Some(Utc::now().timestamp() + 1)).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert!(
        body_text(refused)
            .await
            .contains("dispatch_identity_unavailable")
    );
    patch(&f, doc! {"expires_at":bson::DateTime::from_millis(0)}).await;
    assert!(
        body_text(lookup(&f, "lost-old-key", Some(Utc::now().timestamp() + 1)).await)
            .await
            .contains("not_received")
    );
}

#[tokio::test]
async fn gateway_receipt_backoff_retry_after_and_crash_schedule_are_bounded() {
    let f = pending("gateway_receipt_backoff", 409, "routed_reply_in_progress").await;
    for (attempt, seconds) in [(2, 30), (3, 60), (4, 120), (5, 300), (6, 300)] {
        due(&f).await;
        late_delivery::sweep(&f.state).await.unwrap();
        let row = raw(&f).await;
        let d = row.get_document("delivery").unwrap();
        let wait = d
            .get_document("preparation")
            .unwrap()
            .get_document("gateway_receipt")
            .unwrap();
        assert_eq!(
            wait.get_i64("attempts")
                .ok()
                .or_else(|| wait.get_i32("attempts").ok().map(i64::from)),
            Some(attempt)
        );
        let delay = d.get_datetime("receipt_due_at").unwrap().timestamp_millis()
            - d.get_datetime("checked_at").unwrap().timestamp_millis();
        assert_eq!(delay, seconds * 1000);
    }
    f.gateway.reset().await;
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("retry-after", "900")
                .set_body_json(json!({"detail":"routed_reply_in_progress"})),
        )
        .mount(&f.gateway)
        .await;
    due(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    let row = raw(&f).await;
    let d = row.get_document("delivery").unwrap();
    let delay = d.get_datetime("receipt_due_at").unwrap().timestamp_millis()
        - d.get_datetime("checked_at").unwrap().timestamp_millis();
    assert!((899_000..=900_000).contains(&delay));
    due(&f).await;
    prep::gateway::acquire(&f.state.db, &f.event.id, "crashed")
        .await
        .unwrap()
        .unwrap();
    let row = raw(&f).await;
    let d = row.get_document("delivery").unwrap();
    let p = d.get_document("preparation").unwrap();
    assert_eq!(
        d.get_datetime("receipt_due_at").unwrap().timestamp_millis()
            - p.get_datetime("lease_until").unwrap().timestamp_millis(),
        180_000
    );
    // A server lower bound beyond the absolute deadline wakes only to settle.
    patch(&f,doc! {"delivery.preparation.gateway_receipt.deadline":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::seconds(5))}).await;
    due(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    let row = raw(&f).await;
    let d = row.get_document("delivery").unwrap();
    assert_eq!(
        d.get_datetime("receipt_due_at").unwrap(),
        d.get_document("preparation")
            .unwrap()
            .get_document("gateway_receipt")
            .unwrap()
            .get_datetime("deadline")
            .unwrap()
    );
    assert_eq!(
        outcomes(&f).await.len(),
        1,
        "only the admission is audited while pending"
    );
}

#[tokio::test]
async fn gateway_reply_target_mint_race_uses_only_committed_target() {
    let f = fixture("gateway_reply_target_race").await;
    settled(&f, false).await;
    patch(&f, doc! {"delivery.state":"pending"}).await;
    f.gateway.reset().await;
    let minted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = minted.clone();
    Mock::given(method("POST")).and(path("/events/cmaeg1.ev.original/reply-target"))
        .respond_with(move |_: &wiremock::Request| {
            let n=count.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200).set_delay(Duration::from_millis(30))
                .set_body_json(json!({"reply_target_ref":format!("cmaeg1.rt.minted-{n}"),"expires_at":(Utc::now()+ChronoDuration::hours(1)).timestamp()}))
        }).mount(&f.gateway).await;
    Mock::given(method("POST"))
        .and(wiremock::matchers::path_regex(
            "^/reply-targets/.*?/messages$",
        ))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(json!({"detail":"routed_reply_in_progress"})),
        )
        .mount(&f.gateway)
        .await;
    tokio::join!(
        late_delivery::process(&f.state, &f.event.id),
        late_delivery::process(&f.state, &f.event.id)
    );
    let event = f
        .state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&f.event.id})
        .await
        .unwrap()
        .unwrap();
    let d = event.delivery.as_ref().unwrap();
    let target_bytes = event_record::delivery_target(
        &f.state.encryption_keys,
        &event,
        d,
        d.target_ciphertext.as_ref().unwrap(),
    )
    .await
    .unwrap();
    let target = std::str::from_utf8(&target_bytes).unwrap();
    assert_eq!(status(&f).await, "unknown");
    let first = sends(&f).await;
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].url.path(),
        format!("/reply-targets/{target}/messages")
    );
    due(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    let all = sends(&f).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].url.path(), all[1].url.path());
    assert_eq!(all[0].body, all[1].body);
    assert_eq!(
        all[0].headers.get("idempotency-key"),
        all[1].headers.get("idempotency-key")
    );
    assert!(minted.load(std::sync::atomic::Ordering::SeqCst) <= 2);
}

#[tokio::test]
async fn gateway_dispatch_shape_matrix_retention_and_foreign_scope() {
    use crate::services::nyxbot_dispatch_identity::{Lookup, lookup as observe};
    let mut f = fixture("gateway_dispatch_shape_matrix").await;
    for shape in ["ordinary", "native", "custom"] {
        f.channel.platform = "slack".into();
        f.channel.gateway_source =
            (shape == "native").then(|| crate::models::nyxbot_channel::GatewaySource::Discord {
                bot_id: "bot".into(),
            });
        let custom = (shape == "custom").then_some("binding");
        patch(&f,doc! {"custom_binding_id":custom,
            "dispatch_identity":bson::to_bson(&dispatch_identity::admission(custom.unwrap_or(&f.channel.id),"key")).unwrap(),
            "expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(6)),
        }).await;
        for (execution, delivery, expected) in [
            ("queued", "pending", "in_progress"),
            ("retryable", "pending", "in_progress"),
            ("completed", "pending", "finished"),
            ("failed", "sending", "finished"),
            ("completed", "sent", "finished"),
            ("failed", "unknown", "finished"),
        ] {
            patch(&f, doc! {"status":execution,"delivery.state":delivery}).await;
            let Lookup::Found(found) = observe(
                &f.state.db,
                &f.state.encryption_keys,
                &f.channel,
                "key",
                custom,
                Some(Utc::now() - ChronoDuration::days(2)),
                Some(Utc::now() - ChronoDuration::days(3)),
            )
            .await
            .unwrap() else {
                panic!("known alias refused for {shape}");
            };
            let value = serde_json::to_value(found).unwrap();
            assert_eq!(value["state"], expected, "{shape}");
            if expected == "finished" {
                assert_eq!(value["outcome"], execution);
                assert_eq!(value["stored_response"], false);
                let provider = if shape != "ordinary" {
                    "none"
                } else {
                    match delivery {
                        "sending" => "started",
                        "sent" => "confirmed",
                        "unknown" => "unconfirmed",
                        _ => "none",
                    }
                };
                assert_eq!(value["provider_delivery"], provider, "{shape}");
            }
        }
        patch(&f, doc! {"expires_at":bson::DateTime::from_millis(0)}).await;
        let Lookup::Found(expired) = observe(
            &f.state.db,
            &f.state.encryption_keys,
            &f.channel,
            "key",
            custom,
            None,
            None,
        )
        .await
        .unwrap() else {
            panic!("retained expiry refused");
        };
        assert_eq!(serde_json::to_value(expired).unwrap()["state"], "expired");
    }
    f.channel.gateway_source = None;
    f.channel.id = "another-channel".into();
    let Lookup::Found(absent) = observe(
        &f.state.db,
        &f.state.encryption_keys,
        &f.channel,
        "key",
        None,
        None,
        None,
    )
    .await
    .unwrap() else {
        panic!("ordinary foreign identity must be absent");
    };
    assert_eq!(
        serde_json::to_value(absent).unwrap(),
        json!({"state":"not_received"})
    );
    let Lookup::Found(expired) = observe(
        &f.state.db,
        &f.state.encryption_keys,
        &f.channel,
        "key",
        None,
        Some(Utc::now() - ChronoDuration::hours(25)),
        None,
    )
    .await
    .unwrap() else {
        panic!("ordinary expiry");
    };
    assert_eq!(
        serde_json::to_value(expired).unwrap(),
        json!({"state":"expired"})
    );
}

#[path = "nyxbot_gateway_reask_tests.rs"]
mod reask_tests;

#[tokio::test]
async fn gateway_reply_health_distinguishes_stored_preparation_from_observed_output() {
    use crate::services::nyxbot_response_recovery as recovery;
    let mut f = fixture("stored_answer_health_2460").await;
    let stored = pending_recovery(&mut f).await;
    recovery::finish(
        &f.state.db,
        &f.state.encryption_keys,
        &stored,
        Some("Stored test answer"),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        answer_health(&f).await["answer_delivery"]["state"],
        "prepared"
    );
    let response = Box::pin(native_responses::snapshot(
        State(f.state.clone()),
        Path(stored.response_id),
        bearer(&f.key),
    ))
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("Stored test answer"));
    let health = answer_health(&f).await;
    assert_eq!(health["answer_delivery"]["state"], "unconfirmed");
    assert!(
        !health["answer_delivery"]
            .to_string()
            .contains("Stored test answer")
    );
    assert_eq!(
        health["answer_delivery"]["reason"],
        "gateway_does_not_confirm"
    );
    assert_eq!(health["recommended_action"], "none");
}
