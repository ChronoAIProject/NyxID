use super::*;

fn cae() -> SecurityEffectIntent {
    new_intent(
        Uuid::new_v4().to_string(),
        SecurityEffect::Cae {
            binding_id: "missing".into(),
            client_id: "missing".into(),
            revoke_source: "user".into(),
        },
    )
}

#[tokio::test]
async fn review_terminal_retention_is_30_days_and_pending_work_never_expires() {
    let db = fixture("intent_retention").await;
    let indexes: Vec<_> = db
        .collection::<Document>(COLLECTION_NAME)
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let ttl = indexes
        .iter()
        .find(|index| index.options.as_ref().unwrap().name.as_deref() == Some(TERMINAL_TTL_INDEX))
        .unwrap();
    assert_eq!(ttl.keys, doc! {"completed_at":1});
    let options = ttl.options.as_ref().unwrap();
    assert_eq!(options.expire_after, Some(Duration::from_secs(30 * 86400)));
    assert_eq!(
        options.partial_filter_expression,
        Some(
            doc! {"version":1,"state":{"$in":["delivered","failed","uncertain"]},"completed_at":{"$type":"date"}}
        )
    );
    for state in ["delivered", "failed", "uncertain"] {
        let row = cae();
        enqueue(&db, &row).await.unwrap();
        let owned = claim(&db, &row).await.unwrap().unwrap();
        let outcome = match state {
            "delivered" => Outcome::Delivered,
            "failed" => Outcome::Failed("fixture"),
            _ => Outcome::Uncertain("fixture"),
        };
        record_outcome(&db, &owned, outcome).await.unwrap();
        let projected = db
            .collection::<Document>(COLLECTION_NAME)
            .aggregate([
                doc! {"$match":{"_id":&row.id}},
                doc! {"$project":{"age":{"$subtract":["$$NOW","$completed_at"]}}},
            ])
            .await
            .unwrap()
            .try_next()
            .await
            .unwrap()
            .unwrap();
        assert!((0..10_000).contains(&projected.get_i64("age").unwrap()));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn review_cae_without_webhook_is_deleted_without_sending() {
    let db = fixture("intent_no_destination").await;
    let app = crate::test_utils::test_app_state(db.clone());
    let server = wiremock::MockServer::start().await;
    let row = cae_occurrence(&db, &app, *server.address()).await;
    db.collection::<Document>("oauth_clients")
        .update_one(
            doc! {"_id":"client-fixture"},
            doc! {"$unset":{"revocation_webhook_url":""}},
        )
        .await
        .unwrap();
    sweep(&db, &app.encryption_keys).await.unwrap();
    assert!(
        db.collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":row.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn review_unknown_version_and_kind_are_untouched_while_known_work_runs() {
    let db = fixture("intent_future_version").await;
    let unknown = doc! {"_id":"future-version","version":2,"kind":"future-effect","state":"pending","next_attempt_at":bson::DateTime::from_millis(0)};
    let unknown_kind = doc! {"_id":"future-kind","version":1,"kind":"future-effect","state":"pending","next_attempt_at":bson::DateTime::from_millis(0)};
    db.collection::<Document>(COLLECTION_NAME)
        .insert_many([&unknown, &unknown_kind])
        .await
        .unwrap();
    let row = occurrence();
    enqueue(&db, &row).await.unwrap();
    let app = crate::test_utils::test_app_state(db.clone());
    sweep(&db, &app.encryption_keys).await.unwrap();
    for before in [&unknown, &unknown_kind] {
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .find_one(doc! {"_id":before.get_str("_id").unwrap()})
                .await
                .unwrap()
                .as_ref(),
            Some(before)
        );
    }
    assert_eq!(
        db.collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn review_claim_and_receipt_errors_do_not_starve_later_intents() {
    for stage in ["claim", "receipt"] {
        let db = fixture("intent_error_continuation").await;
        let mut broken = cae();
        broken.id = "a-broken".into();
        let mut working = cae();
        working.id = "z-working".into();
        enqueue(&db, &broken).await.unwrap();
        enqueue(&db, &working).await.unwrap();
        let disallowed = if stage == "claim" {
            "claimed"
        } else {
            "failed"
        };
        db.run_command(doc! {"collMod":COLLECTION_NAME,"validator":{"$nor":[{"_id":&broken.id,"state":disallowed}]}}).await.unwrap();
        let app = crate::test_utils::test_app_state(db.clone());
        sweep(&db, &app.encryption_keys).await.unwrap();
        assert_eq!(saved_intent(&db, &working).await.state, IntentState::Failed);
        assert_eq!(
            saved_intent(&db, &broken).await.state,
            if stage == "claim" {
                IntentState::Pending
            } else {
                IntentState::Claimed
            }
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn review_terminal_transitions_log_once_even_when_replayed() {
    // Production installs one process-wide subscriber before workers start.
    // Use an isolated process to test that contract without replacing the
    // other libtest threads' dispatchers or racing their callsite caches.
    const CHILD: &str = "NYXID_SECURITY_EFFECT_LOG_TEST_CHILD";
    if std::env::var(CHILD).as_deref() != Ok("1") {
        let output = tokio::process::Command::new(std::env::current_exe().unwrap())
            .env(CHILD, "1")
            .args([
                "services::security_effect_service::tests::review_tests::review_terminal_transitions_log_once_even_when_replayed",
                "--exact", "--test-threads=1", "--nocapture",
            ])
            .output().await.unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tracing_subscriber::prelude::*;
    struct Errors(Arc<AtomicUsize>);
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Errors {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if event
                .metadata()
                .target()
                .ends_with("security_effect_service")
                && *event.metadata().level() == tracing::Level::ERROR
            {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    let db = fixture("intent_transition_logs").await;
    let errors = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(Errors(errors.clone()));
    tracing::subscriber::set_global_default(subscriber).unwrap();
    {
        for state in ["failed", "uncertain", "abandoned"] {
            let row = cae();
            enqueue(&db, &row).await.unwrap();
            let owned = claim(&db, &row).await.unwrap().unwrap();
            if state == "abandoned" {
                assert!(begin_send(&db, &owned).await.unwrap());
                expire_claim(&db, &row).await;
                let sending = saved_intent(&db, &row).await;
                for _ in 0..3 {
                    abandon_uncertain(&db, &sending).await.unwrap();
                }
            } else {
                for _ in 0..3 {
                    record_outcome(
                        &db,
                        &owned,
                        if state == "failed" {
                            Outcome::Failed("fixture")
                        } else {
                            Outcome::Uncertain("fixture")
                        },
                    )
                    .await
                    .unwrap();
                }
            }
        }
    }
    assert_eq!(errors.load(Ordering::SeqCst), 3);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn review_integrity_is_bounded_read_only_metadata_and_shows_stuck_work() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(Mutex::new(Vec::new()));
    let capture = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "intent_diagnostics",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                capture.lock().unwrap().push(event.command);
            }
        }),
    )
    .await;
    ensure_indexes(&db).await.unwrap();
    let mut pending = occurrence();
    pending.created_at = Utc::now() - chrono::Duration::hours(2);
    let mut failed = cae();
    failed.state = IntentState::Failed;
    let mut uncertain = cae();
    uncertain.state = IntentState::Uncertain;
    for row in [&pending, &failed, &uncertain] {
        enqueue(&db, row).await.unwrap();
    }
    let many: Vec<_> = (0..1005)
        .map(|_| {
            let mut r = cae();
            r.state = IntentState::Delivered;
            r
        })
        .collect();
    db.collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .insert_many(many)
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let health = diagnostics(&db).await.unwrap();
    assert_eq!(health.counts.len(), 4);
    let delivered = health
        .counts
        .iter()
        .find(|c| c.state == "delivered")
        .unwrap();
    assert_eq!(delivered.count, 1000);
    assert!(delivered.at_least);
    assert!((7190..7210).contains(&health.oldest_pending_age_seconds.unwrap()));
    let json = serde_json::to_string(&health).unwrap();
    for forbidden in ["resource_id", "binding_id", "client_id", "event", "fixture"] {
        assert!(!json.contains(forbidden), "{json}");
    }
    let commands = commands.lock().unwrap().clone();
    assert_eq!(commands.len(), 12);
    for command in &commands {
        assert_eq!(command.get_str("aggregate").unwrap(), COLLECTION_NAME);
        assert_eq!(command.get_str("hint").unwrap(), DIAGNOSTIC_INDEX);
        assert!(command.contains_key("maxTimeMS"));
        let stages = command.get_array("pipeline").unwrap();
        assert!(
            stages
                .iter()
                .any(|stage| stage.as_document().unwrap().get_i64("$limit") == Ok(1001))
        );
    }
    println!("Integrity diagnostic: {json}");
    db.drop().await.unwrap();
}

#[tokio::test]
async fn review_cae_connects_to_the_pinned_answer_not_rebound_dns_or_redirect() {
    use reqwest::dns::{Addrs, Name, Resolve, Resolving};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Rebound {
        approved: std::net::SocketAddr,
        rebound: std::net::SocketAddr,
        calls: Arc<AtomicUsize>,
    }
    impl Resolve for Rebound {
        fn resolve(&self, _: Name) -> Resolving {
            let address = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.approved
            } else {
                self.rebound
            };
            Box::pin(async move { Ok(Box::new([address].into_iter()) as Addrs) })
        }
    }
    let approved = wiremock::MockServer::start().await;
    let internal = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/hook", internal.uri())),
        )
        .mount(&approved)
        .await;
    let resolutions = Arc::new(AtomicUsize::new(0));
    // The validated answer is the first fixture. Any DNS lookup at connect
    // returns the second fixture, representing the rebound internal address.
    let resolver = Arc::new(Rebound {
        approved: *approved.address(),
        rebound: *internal.address(),
        calls: resolutions.clone(),
    });
    let validated_addresses: Vec<_> = resolver
        .resolve("cae-rebinding.invalid".parse().unwrap())
        .await
        .unwrap()
        .collect();
    let target = "http://cae-rebinding.invalid/hook".to_owned();
    let connect_resolver = resolver.clone();
    let fixture = crate::services::channel_media_service::TestPinnedTransport {
        target: target.clone(),
        host: "cae-rebinding.invalid".into(),
        addresses: validated_addresses,
        builder: Arc::new(move || {
            reqwest::Client::builder().dns_resolver(connect_resolver.clone())
        }),
    };
    let db = self::fixture("cae_dns_pin").await;
    let app = crate::test_utils::test_app_state(db.clone());
    let row = cae_occurrence(&db, &app, *approved.address()).await;
    db.collection::<Document>("oauth_clients")
        .update_one(
            doc! {"_id":"client-fixture"},
            doc! {"$set":{"revocation_webhook_url":&target}},
        )
        .await
        .unwrap();
    crate::services::channel_media_service::TEST_PINNED_TRANSPORT
        .scope(fixture, sweep(&db, &app.encryption_keys))
        .await
        .unwrap();
    assert_eq!(approved.received_requests().await.unwrap().len(), 1);
    assert_eq!(internal.received_requests().await.unwrap().len(), 0);
    assert_eq!(
        resolutions.load(Ordering::SeqCst),
        1,
        "only the validation lookup runs; connect must not resolve again"
    );
    assert_eq!(
        saved_intent(&db, &row).await.state,
        IntentState::Failed,
        "redirect is not followed"
    );
    // Control: without pinning, this same resolver now connects to the other
    // fixture. No public host or actual internal service participates.
    reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(resolver)
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
        .post(&target)
        .send()
        .await
        .unwrap();
    assert_eq!(internal.received_requests().await.unwrap().len(), 1);
    assert_eq!(resolutions.load(Ordering::SeqCst), 2);
    db.drop().await.unwrap();
}
