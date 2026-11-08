//! Real Mongo transactions, real schedule admission and local TLS push transport.
use super::*;
use crate::models::notification_channel::COLLECTION_NAME as CHANNELS;
use crate::services::trigger_schedule::{
    notification_delivery as receipt, result_delivery as delivery,
};

async fn enable_push(state: &mut AppState) {
    state.config.fcm_project_id = Some("test".into());
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        json!({
            "client_email":"fixture@example.test",
            "private_key":crate::test_utils::TEST_GCP_SA_PRIVATE_KEY,
        })
        .to_string(),
    )
    .unwrap();
    state.fcm_auth = Some(Arc::new(
        crate::services::push_service::FcmAuth::from_service_account_file(
            file.path().to_str().unwrap(),
        )
        .unwrap(),
    ));
    let now = mongodb::bson::DateTime::now();
    state.db.collection::<Document>(CHANNELS).insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"user_id":OWNER,"push_enabled":true,"telegram_enabled":true,"telegram_chat_id":42_i64,
        "push_devices":[{"device_id":"device","platform":"fcm","token":"test-device-token","registered_at":now}],
        "created_at":now,"updated_at":now,
    }).await.unwrap();
}

async fn start(state: &AppState) -> Trigger {
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let mut trigger = automation(state, &agent.id, Some(ThreadPolicy::Dedicated)).await;
    if let TriggerDelivery::Assistant { deliver_to, .. } = &mut trigger.delivery {
        *deliver_to = DeliverTo::Notification;
    }
    state
        .db
        .collection::<Trigger>(TRIGGERS)
        .replace_one(doc! {"_id":&trigger.id}, &trigger)
        .await
        .unwrap();
    make_due(state, &trigger).await;
    tick(state).await.unwrap();
    trigger
}

async fn reload(state: &AppState, id: &str) -> TriggerRun {
    state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

async fn wait_retry(state: &AppState, trigger: &Trigger) -> TriggerRun {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if let Some(run) = state.db.collection::<TriggerRun>(RUNS).find_one(doc! {"trigger_id":&trigger.id,"notification_delivery.state":"retrying","result_delivery.lease_until":{"$lte":schedules::date(Utc::now())}}).await.unwrap()
                && state.db.collection::<Document>(schedules::WORK).find_one(doc! {"_id":&run.id,"at":{"$gte":schedules::date(run.notification_delivery.as_ref().unwrap().retry_at.unwrap())}}).await.unwrap().is_some() {
                return run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap()
}

async fn retry_due(state: &AppState, run: &TriggerRun) {
    let before = schedules::date(Utc::now() - Duration::seconds(1));
    state.db.collection::<Document>(RUNS).update_one(doc! {"_id":&run.id}, doc! {"$set":{
        "result_delivery.lease_until":before,"result_delivery.retry_at":before,"notification_delivery.retry_at":before,
    }}).await.unwrap();
    // Recovery reads the original saved assistant reply, never admits a turn.
    run_job_once(state, &doc! {"_id":&run.id,"kind":"run"})
        .await
        .unwrap();
}

async fn assert_notice(state: &AppState, run: &TriggerRun, code: &str) {
    assert_eq!(run.outcome, RunOutcome::Failed);
    assert_eq!(run.execution_outcome, Some(RunOutcome::Completed));
    assert_eq!(
        run.notification_delivery
            .as_ref()
            .unwrap()
            .reason
            .as_deref(),
        Some(code)
    );
    let thread = run.thread_id.as_deref().unwrap();
    let messages = engine::messages(&state.db, OWNER, thread, 100, None)
        .await
        .unwrap();
    assert!(
        messages
            .iter()
            .any(|m| m.role == "assistant" && m.status == "completed" && m.text == "Work finished")
    );
    let notices: Vec<_> = messages
        .iter()
        .filter(|m| m.role == "event" && m.text.contains("Mobile push delivery needs attention"))
        .collect();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].text.contains("Task completed"));
    assert!(notices[0].text.contains("/assistant/automations"));
    assert!(notices[0].text.contains("Telegram"));
    let response =
        serde_json::to_value(crate::handlers::triggers::run_response(run.clone())).unwrap();
    assert_eq!(response["execution_outcome"], "completed");
    assert!(
        response["notification_delivery"]["message"]
            .as_str()
            .unwrap()
            .len()
            > 30
    );
    for secret in [
        "test-device-token",
        "PRIVATE_PROVIDER_TEXT",
        "secret-report",
        "claim_id",
        "Work finished",
    ] {
        assert!(!response.to_string().contains(secret));
        assert!(!notices[0].text.contains(secret));
    }
}

#[tokio::test]
async fn schedule_notification_dedicated_result_accepted_once_and_retained() {
    let (mut state, calls, agent_server) = setup("push_dedicated_success").await;
    let (client, requests, server) = result_transport().await;
    state.http_client = client;
    enable_push(&mut state).await;
    let trigger = start(&state).await;
    let run = completed(&state, &trigger).await;
    assert_eq!(run.outcome, RunOutcome::Completed);
    assert_eq!(run.execution_outcome, Some(RunOutcome::Completed));
    assert_eq!(
        run.notification_delivery.as_ref().unwrap().state,
        "accepted"
    );
    assert_eq!(calls.lock().await.len(), 1);
    let thread = engine::get(&state.db, OWNER, run.thread_id.as_deref().unwrap())
        .await
        .unwrap();
    settle_result(&state, &thread, &run, "Must not resend", None)
        .await
        .unwrap();
    run_job_once(&state, &doc! {"_id":&run.id,"kind":"run"})
        .await
        .unwrap();
    let sent = requests.lock().await;
    assert_eq!(sent.len(), 1);
    assert!(sent[0].0.contains("messages:send"));
    assert_eq!(
        sent[0].1["message"]["notification"]["body"],
        "Work finished"
    );
    assert_eq!(sent[0].1["message"]["data"]["conversation_id"], thread.id);
    assert_eq!(
        sent[0].1["message"]["data"]["deeplink"],
        crate::services::assistant_links::AssistantPage::Conversation {
            conversation: &thread.id
        }
        .url(&state.config.frontend_url)
    );
    assert_eq!(calls.lock().await.len(), 1);
    server.abort();
    agent_server.abort();
}

#[tokio::test]
async fn schedule_notification_availability_is_actionable_and_telegram_is_not_push() {
    let (mut state, calls, agent_server) = setup("push_availability").await;
    for (index, code) in [
        "push_not_set_up",
        "push_disabled",
        "push_no_devices",
        "push_provider_unavailable",
    ]
    .into_iter()
    .enumerate()
    {
        if index == 1 {
            enable_push(&mut state).await;
            state
                .db
                .collection::<Document>(CHANNELS)
                .update_one(doc! {"user_id":OWNER}, doc! {"$set":{"push_enabled":false}})
                .await
                .unwrap();
        } else if index == 2 {
            state
                .db
                .collection::<Document>(CHANNELS)
                .update_one(
                    doc! {"user_id":OWNER},
                    doc! {"$set":{"push_enabled":true,"push_devices":[]}},
                )
                .await
                .unwrap();
        } else if index == 3 {
            state
                .db
                .collection::<Document>(CHANNELS)
                .delete_many(doc! {})
                .await
                .unwrap();
            enable_push(&mut state).await;
            state.fcm_auth = None;
        }
        let trigger = start(&state).await;
        let run = completed(&state, &trigger).await;
        assert_notice(&state, &run, code).await;
        assert_eq!(run.notification_delivery.as_ref().unwrap().attempts, 0);
        let thread = engine::get(&state.db, OWNER, run.thread_id.as_deref().unwrap())
            .await
            .unwrap();
        settle_result(&state, &thread, &run, "Do not repeat", None)
            .await
            .unwrap();
        assert_notice(&state, &reload(&state, &run.id).await, code).await;
    }
    assert_eq!(calls.lock().await.len(), 4);
    agent_server.abort();
}

#[tokio::test]
async fn schedule_notification_refusals_and_lost_responses_are_not_success() {
    for (status, code, delivery_state) in [
        (403, "push_provider_rejected", "failed"),
        (404, "push_devices_invalid", "failed"),
        (0, "push_delivery_unknown", "unknown"),
    ] {
        let (mut state, calls, agent_server) = setup(&format!("push_refusal_{status}")).await;
        let (client, requests, server) = result_transport_responses(vec![(status, None)]).await;
        state.http_client = client;
        enable_push(&mut state).await;
        let trigger = start(&state).await;
        let run = completed(&state, &trigger).await;
        assert_notice(&state, &run, code).await;
        assert_eq!(
            run.notification_delivery.as_ref().unwrap().state,
            delivery_state
        );
        run_job_once(&state, &doc! {"_id":&run.id,"kind":"run"})
            .await
            .unwrap();
        assert_eq!(calls.lock().await.len(), 1);
        assert_eq!(requests.lock().await.len(), 1);
        if status == 404 {
            let channel = state
                .db
                .collection::<Document>(CHANNELS)
                .find_one(doc! {"user_id":OWNER})
                .await
                .unwrap()
                .unwrap();
            assert!(channel.get_array("push_devices").unwrap().is_empty());
        }
        server.abort();
        agent_server.abort();
    }
}

#[tokio::test]
async fn schedule_notification_retries_saved_report_after_explicit_refusal_only() {
    let (mut state, calls, agent_server) = setup("push_retry_success").await;
    let (client, requests, server) =
        result_transport_responses(vec![(429, Some(120)), (200, None)]).await;
    state.http_client = client;
    enable_push(&mut state).await;
    let trigger = start(&state).await;
    let run = wait_retry(&state, &trigger).await;
    assert_eq!(run.execution_outcome, Some(RunOutcome::Completed));
    assert_eq!(run.outcome, RunOutcome::Started);
    let projected = state
        .db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! {"_id":&trigger.id})
        .await
        .unwrap()
        .unwrap()
        .schedule_state
        .last_run
        .unwrap();
    assert_eq!(
        projected.notification_delivery.as_ref().unwrap().state,
        "retrying"
    );
    assert_eq!(projected.execution_outcome, Some(RunOutcome::Completed));
    assert!(
        run.notification_delivery
            .as_ref()
            .unwrap()
            .retry_at
            .unwrap()
            > Utc::now() + Duration::seconds(100)
    );
    run_job_once(&state, &doc! {"_id":&run.id,"kind":"run"})
        .await
        .unwrap();
    assert_eq!(requests.lock().await.len(), 1);
    // Legitimate overlap remains blocked while delivery is pending; no task is cleared.
    let blocked = schedules::enqueue_now(&state.db, &trigger).await.unwrap();
    let job = schedules::claim(&state.db, &blocked.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    run_job_once(&state, &job).await.unwrap();
    let blocked = reload(&state, &blocked.id).await;
    assert_eq!(blocked.outcome, RunOutcome::Skipped);
    assert_eq!(blocked.reason.as_deref(), Some("overlap_skip"));
    retry_due(&state, &run).await;
    // The newer skipped occurrence owns the last-run projection. Inspect the
    // original run directly after its synchronous recovery has settled.
    let saved = reload(&state, &run.id).await;
    assert_eq!(saved.outcome, RunOutcome::Completed);
    assert_eq!(saved.notification_delivery.as_ref().unwrap().attempts, 2);
    assert_eq!(calls.lock().await.len(), 1);
    let sent = requests.lock().await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].1, sent[1].1);
    server.abort();
    agent_server.abort();
}

#[tokio::test]
async fn schedule_notification_retry_budget_and_live_preferences_stop_delivery() {
    for disabled in [false, true] {
        let (mut state, calls, agent_server) = setup(&format!("push_retry_bound_{disabled}")).await;
        let (client, requests, server) = result_transport_responses(vec![(503, None); 3]).await;
        state.http_client = client;
        enable_push(&mut state).await;
        let trigger = start(&state).await;
        let run = wait_retry(&state, &trigger).await;
        if disabled {
            state
                .db
                .collection::<Document>(CHANNELS)
                .update_one(doc! {"user_id":OWNER}, doc! {"$set":{"push_enabled":false}})
                .await
                .unwrap();
        }
        retry_due(&state, &run).await;
        if !disabled {
            retry_due(&state, &run).await;
        }
        let saved = completed(&state, &trigger).await;
        assert_notice(
            &state,
            &saved,
            if disabled {
                "push_disabled"
            } else {
                "push_retries_exhausted"
            },
        )
        .await;
        assert_eq!(calls.lock().await.len(), 1);
        assert_eq!(requests.lock().await.len(), if disabled { 1 } else { 3 });
        server.abort();
        agent_server.abort();
    }
}

#[tokio::test]
async fn schedule_notification_lease_protects_live_sender_and_fences_stale_recovery() {
    for prior in ["sending", "accepted", "retrying"] {
        let (mut state, calls, agent_server) = setup(&format!("push_lease_{prior}")).await;
        let (client, requests, server) = result_transport().await;
        state.http_client = client;
        enable_push(&mut state).await;
        let trigger = start(&state).await;
        let run = completed(&state, &trigger).await;
        // Reconstruct the persisted boundary between result generation and
        // final settlement; the original transcript/turn identity is retained.
        state
            .db
            .collection::<Document>(RUNS)
            .update_one(
                doc! {"_id":&run.id},
                doc! {
                    "$set":{"outcome":"started","result_claimed":false},
                    "$unset":{"notification_delivery":"","result_delivery":"","finished_at":""},
                },
            )
            .await
            .unwrap();
        let delivery::Claim::Acquired(claim) = delivery::claim(&state.db, &run).await.unwrap()
        else {
            panic!("claim");
        };
        let mut status = receipt::status(prior, None, 1, None);
        if prior == "retrying" {
            status.retry_at = Some(Utc::now() + Duration::minutes(10));
        }
        assert!(receipt::save(&state.db, &claim, &status).await.unwrap());
        let job = doc! {"_id":&run.id,"kind":"run"};
        let (first, second) = tokio::join!(run_job_once(&state, &job), run_job_once(&state, &job));
        first.unwrap();
        second.unwrap();
        assert_eq!(reload(&state, &run.id).await.outcome, RunOutcome::Started);
        assert_eq!(requests.lock().await.len(), 1);
        let before = schedules::date(Utc::now() - Duration::seconds(1));
        state.db.collection::<Document>(RUNS).update_one(doc! {"_id":&run.id},doc! {"$set":{"result_delivery.lease_until":before,"result_delivery.retry_at":before}}).await.unwrap();
        run_job_once(&state, &doc! {"_id":&run.id,"kind":"run"})
            .await
            .unwrap();
        let saved = reload(&state, &run.id).await;
        match prior {
            "sending" => assert_notice(&state, &saved, "push_delivery_unknown").await,
            "accepted" => assert_eq!(saved.outcome, RunOutcome::Completed),
            "retrying" => {
                assert_eq!(saved.outcome, RunOutcome::Started);
                let work = state
                    .db
                    .collection::<Document>(schedules::WORK)
                    .find_one(doc! {"_id":&run.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(work.get_datetime("at").unwrap().to_chrono() >= status.retry_at.unwrap());
            }
            _ => unreachable!(),
        }
        assert!(
            !receipt::save(
                &state.db,
                &claim,
                &receipt::status("accepted", None, 1, None)
            )
            .await
            .unwrap()
        );
        assert_eq!(requests.lock().await.len(), 1);
        assert_eq!(calls.lock().await.len(), 1);
        server.abort();
        agent_server.abort();
    }
}

#[tokio::test]
async fn schedule_notification_legacy_claim_and_expiry_leave_one_visible_notice() {
    for legacy in [true, false] {
        let (mut state, calls, agent_server) = setup(&format!("push_expiry_{legacy}")).await;
        let (client, requests, server) = result_transport().await;
        state.http_client = client;
        enable_push(&mut state).await;
        let trigger = start(&state).await;
        let run = completed(&state, &trigger).await;
        state
            .db
            .collection::<Document>(RUNS)
            .update_one(
                doc! {"_id":&run.id},
                doc! {
                    "$set":{"outcome":"started","result_claimed":legacy},
                    "$unset":{"notification_delivery":"","result_delivery":"","finished_at":""},
                },
            )
            .await
            .unwrap();
        if !legacy {
            let delivery::Claim::Acquired(claim) = delivery::claim(&state.db, &run).await.unwrap()
            else {
                panic!("claim");
            };
            receipt::save(
                &state.db,
                &claim,
                &receipt::status("retrying", Some("push_temporarily_refused"), 1, Some(503)),
            )
            .await
            .unwrap();
            let before = schedules::date(Utc::now() - Duration::seconds(1));
            state.db.collection::<Document>(RUNS).update_one(doc! {"_id":&run.id},doc! {"$set":{"result_delivery.lease_until":before,"result_delivery.expires_at":before}}).await.unwrap();
        }
        let job = doc! {"_id":&run.id,"kind":"run"};
        run_job_once(&state, &job).await.unwrap();
        run_job_once(&state, &job).await.unwrap();
        assert_notice(
            &state,
            &reload(&state, &run.id).await,
            if legacy {
                "push_delivery_unknown"
            } else {
                "push_retries_exhausted"
            },
        )
        .await;
        assert_eq!(requests.lock().await.len(), 1);
        assert_eq!(calls.lock().await.len(), 1);
        server.abort();
        agent_server.abort();
    }
}

#[tokio::test]
async fn schedule_notification_delivers_downstream_failure_report_without_retrying_partial_acceptance()
 {
    let report = "进度报告：CMA 暂时不可用。已保存检查结果；下一步等待服务恢复。".repeat(12);
    let (mut state, calls, agent_server) = setup_reply("push_downstream_report", &report).await;
    let (client, requests, server) =
        result_transport_responses(vec![(200, None), (503, Some(60))]).await;
    state.http_client = client;
    enable_push(&mut state).await;
    state.db.collection::<Document>(CHANNELS).update_one(doc! {"user_id":OWNER},doc! {"$push":{"push_devices":{
        "device_id":"second-device","platform":"fcm","token":"second-token","registered_at":mongodb::bson::DateTime::now(),
    }}}).await.unwrap();
    let trigger = start(&state).await;
    let run = completed(&state, &trigger).await;
    assert_eq!(run.outcome, RunOutcome::Completed);
    assert_eq!(
        run.notification_delivery.as_ref().unwrap().state,
        "accepted"
    );
    run_job_once(&state, &doc! {"_id":&run.id,"kind":"run"})
        .await
        .unwrap();
    assert_eq!(requests.lock().await.len(), 2);
    assert_eq!(
        requests.lock().await[0].1["message"]["notification"]["body"],
        report.chars().take(200).collect::<String>()
    );
    let messages = engine::messages(
        &state.db,
        OWNER,
        run.thread_id.as_deref().unwrap(),
        100,
        None,
    )
    .await
    .unwrap();
    assert!(
        messages
            .iter()
            .any(|m| m.role == "assistant" && m.text == report)
    );
    assert_eq!(calls.lock().await.len(), 1);
    server.abort();
    agent_server.abort();
}
