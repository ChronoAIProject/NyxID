use super::*;
use crate::services::shutdown::drain::{Drain, End};
use futures::TryStreamExt;

async fn started_turn(
    state: &AppState,
) -> (
    crate::models::assistant_conversation::AssistantConversation,
    tokio::task::JoinHandle<String>,
) {
    let response = turns(
        State(state.clone()),
        test_auth_user(OWNER),
        turn_request(None),
    )
    .await
    .unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            bytes.extend_from_slice(&body.next().await.unwrap().unwrap());
            if String::from_utf8_lossy(&bytes).contains("Partial answer") {
                break;
            }
        }
    })
    .await
    .unwrap();
    let row = engine::list(&state.db, OWNER, 1, None, None)
        .await
        .unwrap()
        .remove(0);
    let consumer = tokio::spawn(async move {
        let rest = axum::body::to_bytes(Body::from_stream(body), 100_000)
            .await
            .unwrap();
        bytes.extend_from_slice(&rest);
        String::from_utf8(bytes).unwrap()
    });
    (row, consumer)
}

async fn drain_audit_row(state: &AppState, event_type: &str) -> bson::Document {
    let rows: Vec<bson::Document> = state
        .db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type": event_type})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("seq").is_some());
    assert_eq!(rows[0].get_str("entry_hash").unwrap().len(), 64);
    assert_eq!(rows[0].get_str("prev_hash").unwrap().len(), 64);
    rows.into_iter().next().unwrap()
}

async fn drain_audit(state: &AppState) -> bson::Document {
    drain_audit_row(state, "backend_shutdown_drain")
        .await
        .get_document("event_data")
        .unwrap()
        .clone()
}

#[tokio::test]
async fn shutdown_drain_waits_for_slow_turn_complete_reply_without_turn_lost() {
    let (state, _, server) = setup(None, Duration::from_secs(1)).await;
    let (row, consumer) = started_turn(&state).await;
    let drain = Drain::begin(&state, Duration::from_secs(10), "SIGTERM").unwrap();
    assert_eq!(drain.counts.turns.get("user"), Some(&1));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), state.shutdown.idle())
            .await
            .is_err()
    );
    assert_eq!(
        drain
            .wait(&state, drain.deadline, std::future::pending())
            .await,
        End::Idle
    );
    let wire = consumer.await.unwrap();
    assert!(wire.contains("Partial answer completed"));
    assert!(!wire.contains("turn_lost"));
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages[1].error_code.is_none());
    drain
        .audit(
            &state,
            End::Idle,
            Default::default(),
            tokio::time::Instant::now() + Duration::from_secs(3),
        )
        .await;
    let audit = drain_audit(&state).await;
    assert_eq!(audit.get_i64("turns_completed_during_drain").unwrap(), 1);
    assert!(audit.get_array("turns_interrupted").unwrap().is_empty());
    assert_eq!(state.shutdown.tracked_units(), 0);
    server.abort();
}

#[tokio::test]
async fn shutdown_deadline_settles_immediately_old_sweep_and_next_send_do_not_settle_twice() {
    let (state, _, server) = setup(None, Duration::from_secs(60)).await;
    let (row, consumer) = started_turn(&state).await;
    let drain = Drain::begin(&state, Duration::from_millis(100), "SIGTERM").unwrap();
    assert_eq!(
        drain
            .wait(&state, drain.deadline, std::future::pending())
            .await,
        End::Deadline
    );
    state.shutdown.interrupt();
    tokio::time::timeout(Duration::from_secs(5), state.shutdown.idle())
        .await
        .unwrap();
    let wire = consumer.await.unwrap();
    assert!(wire.contains("turn_lost"));
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].error_code.as_deref(), Some("turn_lost"));
    assert_eq!(messages[1].error_detail.as_ref().unwrap().stage, "shutdown");
    assert_eq!(engine::sweep_orphaned_turns(&state.db).await.unwrap(), 0);
    assert!(
        crate::services::shutdown::drain::settle_remaining(&state)
            .await
            .unwrap()
            .is_empty()
    );
    let request = engine::TurnRequest {
        conversation_id: Some(row.id.clone()),
        text: "next".into(),
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        model: None,
        access_mode: None,
    };
    engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap();
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.error_code.as_deref() == Some("turn_lost"))
            .count(),
        1
    );
    drain
        .audit(
            &state,
            End::Deadline,
            Default::default(),
            tokio::time::Instant::now() + Duration::from_secs(3),
        )
        .await;
    let audit = drain_audit(&state).await;
    assert_eq!(audit.get_i64("turns_completed_during_drain").unwrap(), 0);
    assert_eq!(
        audit.get_array("turns_interrupted").unwrap(),
        &vec![bson::Bson::String(row.active_turn.unwrap().turn_id)]
    );
    assert!(audit.get_bool("deadline_reached").unwrap());
    server.abort();
}

#[tokio::test]
async fn shutdown_audit_keeps_interrupted_turn_id_when_mongodb_rejects_settlement() {
    let (state, _, server) = setup(None, Duration::from_secs(60)).await;
    let (row, consumer) = started_turn(&state).await;
    state
        .db
        .run_command(doc! {
            "collMod": crate::models::assistant_message::COLLECTION_NAME,
            "validator": {"role": {"$ne": "assistant"}},
            "validationLevel": "strict", "validationAction": "error",
        })
        .await
        .unwrap();
    let drain = Drain::begin(&state, Duration::from_secs(10), "SIGTERM").unwrap();
    state.shutdown.interrupt();
    state.shutdown.stop();
    tokio::time::timeout(Duration::from_secs(3), state.shutdown.execution_stopped())
        .await
        .unwrap();
    assert!(
        engine::settle_shutdown_turn(
            &state.db,
            &row,
            &crate::services::assistant_engine_worker::instance().id,
        )
        .await
        .is_err()
    );
    drain
        .audit(
            &state,
            End::Deadline,
            Default::default(),
            tokio::time::Instant::now() + Duration::from_secs(3),
        )
        .await;
    let audit = drain_audit(&state).await;
    assert_eq!(audit.get_i64("turns_completed_during_drain").unwrap(), 0);
    assert_eq!(
        audit.get_array("turns_interrupted").unwrap(),
        &vec![bson::Bson::String(row.active_turn.unwrap().turn_id)],
    );
    consumer.abort();
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_actual_server_process_sigterm_completes_slow_turn_and_exits_zero() {
    actual_server_process(ProcessDrain::SlowTurn).await;
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_actual_server_idle_node_and_subscriptions_exit_idle_after_grace() {
    actual_server_process(ProcessDrain::IdleObservers).await;
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_actual_server_killed_during_drain_retains_start_audit_only() {
    actual_server_process(ProcessDrain::Killed).await;
}

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessDrain {
    SlowTurn,
    IdleObservers,
    Killed,
}

#[cfg(unix)]
async fn actual_server_process(scenario: ProcessDrain) {
    let idle_observers = scenario == ProcessDrain::IdleObservers;
    use std::process::{Command, Stdio};
    struct Child(CommandChild);
    type CommandChild = std::process::Child;
    impl Drop for Child {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let delay = if scenario == ProcessDrain::Killed {
        600
    } else {
        3
    };
    let (state, _, upstream) = setup(None, Duration::from_secs(delay)).await;
    let directory = tempfile::tempdir().unwrap();
    let private = directory.path().join("private.pem");
    let public = directory.path().join("public.pem");
    assert!(
        Command::new("openssl")
            .args(["genrsa", "-out"])
            .arg(&private)
            .arg("3072")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new("openssl")
            .args(["rsa", "-in"])
            .arg(&private)
            .args(["-pubout", "-out"])
            .arg(&public)
            .output()
            .unwrap()
            .status
            .success()
    );
    let public_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let internal_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = public_listener.local_addr().unwrap().port();
    let internal = internal_listener.local_addr().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let mut database_url =
        url::Url::parse(&std::env::var("NYXID_TEST_DATABASE_URL").unwrap()).unwrap();
    database_url.set_path(state.db.name());
    let output = directory.path().join("server.log");
    let log = std::fs::File::create(&output).unwrap();
    drop((public_listener, internal_listener));
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .current_dir(directory.path())
        .args([
            "--exact",
            "handlers::assistant_nyxagent::tests::shutdown_tests::shutdown_process_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("RUST_MIN_STACK", "8388608")
        .env("DATABASE_URL", database_url.as_str())
        .env("ENVIRONMENT", "development")
        .env("PORT", port.to_string())
        .env("BASE_URL", &base)
        .env("JWT_ISSUER", &base)
        .env("JWT_PRIVATE_KEY_PATH", &private)
        .env("JWT_PUBLIC_KEY_PATH", &public)
        .env("ENCRYPTION_KEY", "11".repeat(32))
        .env("AUDIT_CHAIN_HMAC_KEY", "02".repeat(32))
        .env("BILLING_LEDGER_HMAC_KEY", "03".repeat(32))
        .env("INTERNAL_BIND_ADDR", internal.to_string())
        .env("INTERNAL_ADVERTISE_URL", format!("http://{internal}"))
        .env("INSTANCE_NAME", "drain-process-test")
        .env("SHUTDOWN_DRAIN_TIMEOUT_SECS", "30")
        // All external startup discovery fails locally. Only test loopback
        // MongoDB and mock upstreams are reachable by the HTTP clients.
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log));
    if !idle_observers {
        command
            .env("CHAIN_VERIFY_INTERVAL_SECS", "0")
            .env("BILLING_RECONCILE_INTERVAL_SECS", "0")
            .env("OAUTH_REFRESH_SWEEP_INTERVAL_SECS", "0")
            .env("CHANNEL_POLL_INTERVAL_SECS", "0");
    }
    let mut child = Child(command.spawn().unwrap());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let boot = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "server exited during startup: {}",
                std::fs::read_to_string(&output).unwrap()
            );
            if client
                .get(format!("{base}/ready"))
                .send()
                .await
                .is_ok_and(|r| r.status() == StatusCode::OK)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        boot.is_ok(),
        "server startup timeout: {}",
        std::fs::read_to_string(&output).unwrap()
    );
    let mut config = state.config.clone();
    config.base_url = base.clone();
    config.jwt_issuer = base.clone();
    config.jwt_private_key_path = private.to_string_lossy().into();
    config.jwt_public_key_path = public.to_string_lossy().into();
    let keys = crate::crypto::jwt::JwtKeys::from_config(&config).unwrap();
    if idle_observers {
        idle_node_and_subscriptions(
            &state,
            &mut child.0,
            &client,
            &base,
            &config,
            &keys,
            &output,
        )
        .await;
        upstream.abort();
        return;
    }
    let token = crate::crypto::jwt::generate_access_token(
        &keys,
        &config,
        &Uuid::parse_str(OWNER).unwrap(),
        "proxy",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let response = client
        .post(format!("{base}/api/v1/assistant/nyxagent/turns"))
        .bearer_auth(token)
        .json(&json!({"text":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            bytes.extend_from_slice(&stream.next().await.unwrap().unwrap());
            if String::from_utf8_lossy(&bytes).contains("Partial answer") {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        Command::new("kill")
            .arg("-TERM")
            .arg(child.0.id().to_string())
            .status()
            .unwrap()
            .success()
    );
    if scenario == ProcessDrain::Killed {
        let audits = state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if audits
                    .find_one(doc! {"event_type": "backend_shutdown_drain_started"})
                    .await
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                assert!(child.0.try_wait().unwrap().is_none());
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("start audit must be visible while the turn is still in flight");
        assert!(child.0.try_wait().unwrap().is_none());
        child.0.kill().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.0.wait().unwrap().signal(), Some(9));
        let start = drain_audit_row(&state, "backend_shutdown_drain_started").await;
        let initial = start
            .get_document("event_data")
            .unwrap()
            .get_document("in_flight_at_start")
            .unwrap();
        assert_eq!(
            initial
                .get_document("turns")
                .unwrap()
                .get_i64("user")
                .unwrap(),
            1
        );
        assert_eq!(initial.get_i64("streams").unwrap(), 1);
        assert_eq!(
            audits
                .count_documents(doc! {"event_type": "backend_shutdown_drain"})
                .await
                .unwrap(),
            0
        );
        upstream.abort();
        return;
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if client
                .get(format!("{base}/ready"))
                .send()
                .await
                .unwrap()
                .status()
                == StatusCode::SERVICE_UNAVAILABLE
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client
            .get(format!("{base}/health"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.unwrap());
        }
    })
    .await
    .unwrap();
    let wire = String::from_utf8(bytes).unwrap();
    assert!(wire.contains("Partial answer completed"));
    assert!(!wire.contains("turn_lost"));
    let status = tokio::time::timeout(Duration::from_secs(35), async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        status.success(),
        "server exit: {}",
        std::fs::read_to_string(&output).unwrap()
    );
    let audit = drain_audit(&state).await;
    assert_eq!(audit.get_i64("turns_completed_during_drain").unwrap(), 1);
    assert_eq!(audit.get_i64("tracked_units_at_finish").unwrap(), 0);
    upstream.abort();
}

#[cfg(unix)]
async fn idle_node_and_subscriptions(
    state: &AppState,
    child: &mut std::process::Child,
    client: &reqwest::Client,
    base: &str,
    config: &crate::config::AppConfig,
    keys: &crate::crypto::jwt::JwtKeys,
    output: &std::path::Path,
) {
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message;

    let tokens = crate::services::token_service::create_session_and_issue_tokens(
        &state.db, config, keys, OWNER, None, None,
    )
    .await
    .unwrap();
    let sessions = crate::models::mcp_session::McpSessionStore::with_db(state.db.clone());
    let sid = sessions
        .create_with_parent_session(OWNER, true, true, Some(&tokens.session_id))
        .await
        .unwrap()
        .unwrap();
    assert!(
        sessions
            .send_notification(
                &sid,
                json!({"jsonrpc":"2.0", "method":"notifications/tools/list_changed"}),
            )
            .await
            .unwrap()
    );
    let mcp = client
        .get(format!("{base}/mcp"))
        .bearer_auth(&tokens.access_token)
        .header("mcp-session-id", &sid)
        .header("accept", "text/event-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(mcp.status(), StatusCode::OK);
    let mut mcp = mcp.bytes_stream();
    let mut mcp_wire = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !String::from_utf8_lossy(&mcp_wire).contains("notifications/tools/list_changed") {
            mcp_wire.extend_from_slice(&mcp.next().await.unwrap().unwrap());
        }
    })
    .await
    .unwrap();

    let live = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = client
                .get(format!("{base}/api/v1/assistant/nyxagent/live"))
                .bearer_auth(&tokens.access_token)
                .send()
                .await
                .unwrap();
            if response.status() == StatusCode::OK {
                break response;
            }
            // Wait for the real Mongo change stream to open, without a test seam.
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let mut live = live.bytes_stream();
    let mut live_wire = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !String::from_utf8_lossy(&live_wire).contains("event: ready") {
            live_wire.extend_from_slice(&live.next().await.unwrap().unwrap());
        }
    })
    .await
    .unwrap();

    let node_id = Uuid::new_v4().to_string();
    let node_token = "nyx_nauth_idle_drain_fixture";
    state
        .db
        .collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &node_id, "user_id": OWNER, "name": "idle drain fixture",
            "status": "offline", "auth_token_hash": crate::crypto::token::hash_token(node_token),
            "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let (mut node, _) = tokio_tungstenite::connect_async(format!(
        "{}/api/v1/nodes/ws",
        base.replacen("http://", "ws://", 1)
    ))
    .await
    .unwrap();
    node.send(Message::Text(
        json!({"type":"auth", "node_id":node_id, "token":node_token})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let authenticated = tokio::time::timeout(Duration::from_secs(5), node.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(authenticated.to_text().unwrap()).unwrap()["type"],
        "auth_ok"
    );

    let started = tokio::time::Instant::now();
    assert!(
        std::process::Command::new("kill")
            .arg("-TERM")
            .arg(child.id().to_string())
            .status()
            .unwrap()
            .success()
    );
    let bound = crate::services::shutdown::SUBSCRIPTION_DRAIN_GRACE + Duration::from_secs(5);
    let result = tokio::time::timeout(bound, async {
        tokio::join!(
            async {
                while let Some(chunk) = mcp.next().await {
                    mcp_wire.extend_from_slice(&chunk.expect("MCP stream must end with clean EOF"));
                }
            },
            async {
                while let Some(chunk) = live.next().await {
                    live_wire
                        .extend_from_slice(&chunk.expect("live stream must end with clean EOF"));
                }
            },
            async {
                loop {
                    match node.next().await.expect("node close frame").unwrap() {
                        Message::Close(Some(close)) => {
                            assert_eq!(u16::from(close.code), 1001);
                            break;
                        }
                        Message::Close(None) => panic!("node close must carry going-away code"),
                        _ => {}
                    }
                }
            },
            async {
                loop {
                    if let Some(status) = child.try_wait().unwrap() {
                        assert!(status.success());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            },
        );
    })
    .await;
    assert!(
        result.is_ok(),
        "idle server did not drain within {bound:?}: {}",
        std::fs::read_to_string(output).unwrap()
    );
    assert!(started.elapsed() < bound);
    let start = drain_audit_row(state, "backend_shutdown_drain_started").await;
    let finish = drain_audit_row(state, "backend_shutdown_drain").await;
    assert!(start.get_i64("seq").unwrap() < finish.get_i64("seq").unwrap());
    let start = start.get_document("event_data").unwrap();
    let audit = finish.get_document("event_data").unwrap();
    for field in [
        "generation",
        "instance_name",
        "engine_worker_id",
        "build_commit",
        "signal",
        "started_at",
        "cleanup_reserve_ms",
        "in_flight_at_start",
    ] {
        assert_eq!(
            start.get(field).unwrap(),
            audit.get(field).unwrap(),
            "{field}"
        );
    }
    assert_eq!(start.get_i64("budget_secs").unwrap(), 30);
    assert_eq!(start.get_i64("cleanup_reserve_ms").unwrap(), 3_000);
    assert_eq!(start.get_i64("node_operations_at_start").unwrap(), 0);
    let counts = start.get_document("in_flight_at_start").unwrap();
    assert_eq!(counts.len(), 6);
    assert!(counts.get_i64("detached").unwrap() >= 0);
    assert!(counts.get_i64("background").unwrap() >= 0);
    assert_eq!(audit.get_str("end").unwrap(), "idle");
    assert!(!audit.get_bool("deadline_reached").unwrap());
    assert!(audit.get_array("turns_interrupted").unwrap().is_empty());
    assert_eq!(audit.get_i64("subscriptions_ended").unwrap(), 2);
    assert_eq!(audit.get_i64("turns_completed_during_drain").unwrap(), 0);
    assert_eq!(audit.get_i64("tracked_units_at_finish").unwrap(), 0);
    let initial = audit.get_document("in_flight_at_start").unwrap();
    assert_eq!(initial.get_i64("http_requests").unwrap(), 0);
    assert_eq!(initial.get_i64("streams").unwrap(), 0);
    assert_eq!(initial.get_i64("subscriptions").unwrap(), 2);
    assert!(initial.get_document("turns").unwrap().is_empty());
    let duration = audit.get_i64("duration_ms").unwrap();
    assert!(duration >= crate::services::shutdown::SUBSCRIPTION_DRAIN_GRACE.as_millis() as i64);
    assert!(duration < bound.as_millis() as i64);
    eprintln!(
        "busy-idle process drain: end=idle duration_ms={duration} subscriptions_ended=2 node_close=1001"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "actual server entrypoint launched by the shutdown_actual_server process tests"]
fn shutdown_process_child() {
    crate::server_main(crate::Cli {
        promote_admin: None,
        command: None,
    });
}

#[tokio::test]
async fn shutdown_accepts_new_turn_during_drain_and_waits_for_its_complete_reply() {
    let (state, _, server) = setup(None, Duration::from_millis(200)).await;
    let drain = Drain::begin(&state, Duration::from_secs(10), "SIGTERM").unwrap();
    assert!(drain.counts.turns.is_empty());
    let (_, consumer) = started_turn(&state).await;
    assert_eq!(
        drain
            .wait(&state, drain.deadline, std::future::pending())
            .await,
        End::Idle
    );
    assert!(consumer.await.unwrap().contains("Partial answer completed"));
    assert_eq!(state.shutdown.outcomes(), (1, vec![]));
    assert_eq!(state.shutdown.tracked_units(), 0);
    server.abort();
}

#[tokio::test]
async fn shutdown_fallback_settles_unstarted_turn_preserves_stop_and_fences_replacement_worker() {
    let (state, _, server) = setup(None, Duration::ZERO).await;
    let request = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: None,
        agent_id: None,
        text: "work".into(),
        model: None,
        access_mode: None,
    };
    let worker = crate::services::assistant_engine_worker::instance()
        .id
        .clone();
    let row = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap();
    let turn = row.active_turn.as_ref().unwrap().turn_id.clone();
    let drain = Drain::begin(&state, Duration::from_secs(10), "SIGTERM").unwrap();
    // Exercise the actual coordinator's fallback, without a running task to
    // settle cooperatively. Cleanup and the audit share its bounded budget.
    crate::finish_shutdown(
        &state,
        &drain,
        End::Deadline,
        &tokio_util::sync::CancellationToken::new(),
        tokio::time::Instant::now() + Duration::from_secs(5),
    )
    .await;
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(messages[1].error_code.as_deref(), Some("turn_lost"));
    assert_eq!(messages[1].error_detail.as_ref().unwrap().stage, "shutdown");
    assert_eq!(state.shutdown.outcomes(), (0, vec![turn]));
    assert_eq!(engine::sweep_orphaned_turns(&state.db).await.unwrap(), 0);

    let stopped = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap();
    engine::request_stop(&state.db, OWNER, &stopped.id)
        .await
        .unwrap();
    let error = engine::settle_shutdown_turn(&state.db, &stopped, &worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(error.code, "cancelled");

    let replaced = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &replaced.id},
            doc! {"$set": {"active_turn.worker_instance.id": "successor-worker"}},
        )
        .await
        .unwrap();
    assert!(
        engine::settle_shutdown_turn(&state.db, &replaced, &worker)
            .await
            .unwrap()
            .is_none()
    );
    let current = engine::get(&state.db, OWNER, &replaced.id).await.unwrap();
    assert_eq!(
        current.active_turn.unwrap().worker_instance.unwrap().id,
        "successor-worker"
    );
    server.abort();
}

#[tokio::test]
async fn shutdown_mcp_and_live_subscriptions_end_cleanly_after_grace_with_recoverable_events() {
    use axum::{middleware, routing::get};
    use tower::ServiceExt;
    let (mut state, _, server) = setup(None, Duration::ZERO).await;
    state.mcp_sessions = Arc::new(crate::models::mcp_session::McpSessionStore::with_db(
        state.db.clone(),
    ));
    state.assistant_live.set_open_for_tests(true);
    let tokens = crate::services::token_service::create_session_and_issue_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        OWNER,
        None,
        None,
    )
    .await
    .unwrap();
    let sid = state
        .mcp_sessions
        .create_with_parent_session(OWNER, true, true, Some(&tokens.session_id))
        .await
        .unwrap()
        .unwrap();
    let app = Router::new()
        .route("/mcp", get(crate::handlers::mcp_transport::mcp_get))
        .route(
            "/live",
            get(|State(state): State<AppState>| async move {
                live(State(state), test_auth_user(OWNER)).await
            }),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.shutdown.clone(),
            crate::mw::shutdown::track,
        ));
    let request = |path: &str| {
        axum::http::Request::builder()
            .uri(path)
            .header("mcp-session-id", &sid)
            .header("authorization", format!("Bearer {}", tokens.access_token))
            .body(Body::empty())
            .unwrap()
    };
    let mcp = app.clone().oneshot(request("/mcp")).await.unwrap();
    let live = app.clone().oneshot(request("/live")).await.unwrap();
    assert_eq!(mcp.status(), StatusCode::OK);
    assert_eq!(live.status(), StatusCode::OK);
    assert_eq!(state.shutdown.counts().subscriptions, 2);
    assert_eq!(state.shutdown.counts().streams, 0);
    let mcp_body = tokio::spawn(axum::body::to_bytes(mcp.into_body(), 100_000));
    let live_body = tokio::spawn(axum::body::to_bytes(live.into_body(), 100_000));
    let drain = Drain::begin(&state, Duration::from_secs(30), "SIGTERM").unwrap();
    let event = json!({"jsonrpc":"2.0", "method":"notifications/tools/list_changed"});
    assert!(
        state
            .mcp_sessions
            .send_notification(&sid, event.clone())
            .await
            .unwrap()
    );
    state
        .assistant_live
        .publish(crate::services::assistant_live::LiveEvent::Resync);
    assert!(!mcp_body.is_finished());
    assert!(!live_body.is_finished());
    let grace = crate::services::shutdown::SUBSCRIPTION_DRAIN_GRACE;
    assert_eq!(
        tokio::time::timeout(
            grace + Duration::from_secs(2),
            drain.wait(&state, drain.work_deadline(), std::future::pending())
        )
        .await
        .unwrap(),
        End::Idle
    );
    assert!(drain.started.elapsed() >= grace);
    let mcp_wire = String::from_utf8(mcp_body.await.unwrap().unwrap().to_vec()).unwrap();
    let live_wire = String::from_utf8(live_body.await.unwrap().unwrap().to_vec()).unwrap();
    assert!(mcp_wire.contains("notifications/tools/list_changed"));
    assert!(live_wire.contains("event: ready"));
    assert!(live_wire.contains("event: resync"));
    // A peer reads the durable outbox with the old cursor, even if the closing
    // connection missed the final event. Browser ready/resync re-reads state.
    let peer = crate::models::mcp_session::McpSessionStore::with_db(state.db.clone());
    let replay = peer.notifications_after(&sid, 0).await.unwrap().unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].payload, event);
    assert!(
        peer.notifications_after(&sid, replay[0].sequence)
            .await
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert_eq!(state.shutdown.subscriptions_ended(), 2);
    assert_eq!(state.shutdown.tracked_units(), 0);
    drain
        .audit(
            &state,
            End::Idle,
            Default::default(),
            tokio::time::Instant::now() + Duration::from_secs(3),
        )
        .await;
    assert_eq!(
        drain_audit(&state)
            .await
            .get_i64("subscriptions_ended")
            .unwrap(),
        2
    );
    server.abort();
}

#[tokio::test]
async fn shutdown_turn_sse_is_not_ended_at_subscription_grace() {
    use axum::{middleware, routing::get};
    use tower::ServiceExt;
    let grace = crate::services::shutdown::SUBSCRIPTION_DRAIN_GRACE;
    let (state, _, server) = setup(None, grace + Duration::from_secs(2)).await;
    let app = Router::new()
        .route(
            "/turn",
            get(|State(state): State<AppState>| async move {
                turns(State(state), test_auth_user(OWNER), turn_request(None)).await
            }),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.shutdown.clone(),
            crate::mw::shutdown::track,
        ));
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/turn")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let consumer = tokio::spawn(axum::body::to_bytes(response.into_body(), 100_000));
    let drain = Drain::begin(&state, Duration::from_secs(30), "SIGTERM").unwrap();
    tokio::time::sleep(grace).await;
    assert_eq!(state.shutdown.counts().streams, 1);
    assert_eq!(state.shutdown.subscriptions_ended(), 0);
    assert!(!consumer.is_finished());
    assert_eq!(
        drain
            .wait(&state, drain.work_deadline(), std::future::pending())
            .await,
        End::Idle
    );
    let wire = String::from_utf8(consumer.await.unwrap().unwrap().to_vec()).unwrap();
    assert!(wire.contains("Partial answer completed"));
    assert!(!wire.contains("turn_lost"));
    server.abort();
}

#[tokio::test]
async fn shutdown_new_turn_after_signal_outlives_wait_and_settles_within_runner_grace() {
    let (state, _, server) = setup(None, Duration::from_secs(60)).await;
    let drain = Drain::begin(&state, Duration::from_secs(10), "SIGTERM").unwrap();
    use axum::{middleware, routing::get};
    use tower::ServiceExt;
    let app = Router::new()
        .route(
            "/turn",
            get(|State(state): State<AppState>| async move {
                turns(State(state), test_auth_user(OWNER), turn_request(None)).await
            }),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.shutdown.clone(),
            crate::mw::shutdown::track,
        ));
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/turn")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let mut bytes = Vec::new();
    while !String::from_utf8_lossy(&bytes).contains("Partial answer") {
        bytes.extend_from_slice(
            &tokio::time::timeout(Duration::from_secs(10), body.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        );
    }
    let row = engine::list(&state.db, OWNER, 1, None, None)
        .await
        .unwrap()
        .remove(0);
    let consumer = tokio::spawn(async move {
        let rest = axum::body::to_bytes(Body::from_stream(body), 100_000)
            .await
            .unwrap();
        bytes.extend_from_slice(&rest);
        String::from_utf8(bytes).unwrap()
    });
    assert!(drain.counts.turns.is_empty());
    assert_eq!(state.shutdown.counts().turns.get("user"), Some(&1));
    state.shutdown.interrupt();
    // Production grants remaining/5: one second with the usual cleanup reserve.
    tokio::time::timeout(Duration::from_secs(1), state.shutdown.idle())
        .await
        .unwrap();
    assert!(consumer.await.unwrap().contains("turn_lost"));
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(messages[1].status, "failed");
    assert_eq!(messages[1].error_code.as_deref(), Some("turn_lost"));
    assert_eq!(messages[1].error_detail.as_ref().unwrap().stage, "shutdown");
    assert_eq!(messages[1].text, "Partial answer");
    assert!(
        crate::services::shutdown::drain::settle_remaining(&state)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(engine::sweep_orphaned_turns(&state.db).await.unwrap(), 0);
    server.abort();
}

#[tokio::test]
async fn shutdown_aborted_runner_and_fallback_cannot_both_write_the_reply() {
    let (state, _, server) = setup(None, Duration::from_secs(60)).await;
    let (row, consumer) = started_turn(&state).await;
    let _drain = Drain::begin(&state, Duration::from_secs(10), "SIGTERM").unwrap();
    // Simulate the runner exhausting its ordinary-settlement allowance.
    state.shutdown.interrupt();
    state.shutdown.stop();
    state.shutdown.execution_stopped().await;
    let winners = crate::services::shutdown::drain::settle_remaining(&state)
        .await
        .unwrap();
    assert_eq!(winners.len(), 1);
    assert!(
        crate::services::shutdown::drain::settle_remaining(&state)
            .await
            .unwrap()
            .is_empty()
    );
    // A stale snapshot cannot write even if its future somehow resumes later:
    // the exact conversation/user/active turn predicate no longer matches.
    assert!(
        engine::finish_turn(
            &state.db,
            &row,
            &row.credential_api_key_id,
            &Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: "late result".into(),
                session_id: None,
                response_id: None,
                error: None
            }
        )
        .await
        .is_err()
    );
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(messages.iter().filter(|m| m.role == "assistant").count(), 1);
    assert_eq!(messages[1].error_code.as_deref(), Some("turn_lost"));
    assert_eq!(messages[1].error_detail.as_ref().unwrap().stage, "shutdown");
    assert_eq!(engine::sweep_orphaned_turns(&state.db).await.unwrap(), 0);
    consumer.await.unwrap();
    server.abort();
}
