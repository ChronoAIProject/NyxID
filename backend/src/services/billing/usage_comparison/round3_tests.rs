use super::*;

async fn pass_with(f: &Fixture, provider: &dyn LagoApi) -> AppResult<()> {
    let runtime = cluster_lease_runtime();
    let token = LeaseStore::acquire(
        &f.db,
        super::super::super::reconcile::RECONCILE_LEASE,
        &runtime.holder,
        std::time::Duration::from_secs(365 * 86400),
    )
    .await
    .unwrap()
    .unwrap();
    let result = run(&f.db, provider, &token, &mut Default::default()).await;
    LeaseStore::release(&f.db, &token).await.unwrap();
    result
}

#[tokio::test]
async fn usage_comparison_round3_publication_is_one_target_update() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 0).await;
            let coverage = coverage::refresh(&f.db).await.unwrap();
            let target =
                f.db.collection::<BillingUsageComparison>(COLLECTION_NAME)
                    .find_one(doc! {"_id":"sub"})
                    .await
                    .unwrap()
                    .unwrap();
            let runtime = cluster_lease_runtime();
            let token = LeaseStore::acquire(
                &f.db,
                super::super::super::reconcile::RECONCILE_LEASE,
                &runtime.holder,
                std::time::Duration::from_secs(365 * 86400),
            )
            .await
            .unwrap()
            .unwrap();
            f.commands.lock().unwrap().clear();
            let saved = publication::save(
                &f.db,
                &token,
                &target,
                &coverage,
                Evidence::Waiting("acknowledgements_not_quiet"),
            )
            .await
            .unwrap()
            .unwrap();
            let commands = f.commands.lock().unwrap().clone();
            assert_eq!(
                commands.len(),
                1,
                "publication is one atomic target update: {commands:?}"
            );
            assert_eq!(commands[0].0, "update");
            assert_eq!(commands[0].1.get_str("update"), Ok(COLLECTION_NAME));
            assert!(!commands[0].1.contains_key("autocommit"));
            assert_eq!(saved.observed_at, coverage.observed_at);
            println!(
                "USAGE_PUBLICATION commands=1 transactions=0 lease_writes=0 coverage_writes=0"
            );
            LeaseStore::release(&f.db, &token).await.unwrap();
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round3_pass_has_no_publication_lease_or_coverage_writes() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 0).await;
            f.commands.lock().unwrap().clear();
            f.pass().await.1.unwrap();
            let commands = f.commands.lock().unwrap().clone();
            let lease_writes: Vec<_> = commands
                .iter()
                .filter(|(name, command)| {
                    matches!(name.as_str(), "findAndModify" | "update" | "delete")
                        && command.get_str(name).ok()
                            == Some(crate::models::coordination::LEASE_COLLECTION_NAME)
                })
                .map(|(name, _)| name.as_str())
                .collect();
            assert_eq!(
                lease_writes,
                ["findAndModify", "delete"],
                "only the runner's acquire/release write its lease"
            );
            assert!(
                !commands
                    .iter()
                    .any(|(_, command)| command.contains_key("autocommit"))
            );
            assert!(
                !commands.iter().any(|(name, command)| {
                    name == "update"
                        && command.get_str("update").ok() == Some(CONTROL_COLLECTION)
                        && command.get_array("updates").unwrap().iter().any(|update| {
                            update
                                .as_document()
                                .unwrap()
                                .get_document("q")
                                .unwrap()
                                .get_str("_id")
                                .ok()
                                == Some(COVERAGE_ID)
                        })
                }),
                "publication never updates coverage"
            );
            assert_eq!(f.observation("sub").await.status, "pending");
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round3_http_status_classes_and_backoff() {
    use super::super::super::lago_client::LagoClient;
    use axum::{Router, routing::get};
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            for status in [400, 401, 403, 404, 410, 422, 429, 503] {
                f.target(&status.to_string(), 0).await;
            }
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            let app = Router::new().route(
                "/api/v1/customers/{customer}/current_usage",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<
                        BTreeMap<String, String>,
                    >| {
                        observed.fetch_add(1, Ordering::SeqCst);
                        async move {
                            let status = query["external_subscription_id"].parse::<u16>().unwrap();
                            axum::http::StatusCode::from_u16(status).unwrap()
                        }
                    },
                ),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let client =
                LagoClient::new(format!("http://{address}"), "fixture-key".into()).unwrap();
            assert!(
                matches!(
                    pass_with(&f, &client).await,
                    Err(AppError::BillingRecoveryFailed {
                        failed_units: 3,
                        ..
                    })
                ),
                "401, 403 and 503 must each fail one unit"
            );
            assert_eq!(calls.load(Ordering::SeqCst), 8);
            for (status, expected, wait) in [
                (400, "unsupported", 86400),
                (401, "error", 300),
                (403, "error", 300),
                (404, "unsupported", 86400),
                (410, "unsupported", 86400),
                (422, "unsupported", 86400),
                (429, "waiting", 600),
                (503, "error", 300),
            ] {
                let target =
                    f.db.collection::<BillingUsageComparison>(COLLECTION_NAME)
                        .find_one(doc! {"_id":status.to_string()})
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(
                    target.comparison.as_ref().unwrap().status,
                    expected,
                    "HTTP {status}"
                );
                assert_eq!(
                    target.next_visit_at,
                    Some(initial_now().to_chrono() + chrono::Duration::seconds(wait)),
                    "HTTP {status}"
                );
            }
            assert_eq!(
                f.observation("404").await.reason,
                "subscription_unavailable"
            );
            assert_eq!(
                f.observation("410").await.reason,
                "subscription_unavailable"
            );
            assert_eq!(
                f.observation("400").await.reason,
                "provider_request_rejected"
            );
            assert_eq!(
                f.observation("422").await.reason,
                "provider_request_rejected"
            );
            f.advance(300).await;
            assert!(matches!(
                pass_with(&f, &client).await,
                Err(AppError::BillingRecoveryFailed {
                    failed_units: 3,
                    ..
                })
            ));
            assert_eq!(calls.load(Ordering::SeqCst), 11);
            f.advance(300).await;
            pass_with(&f, &client).await.unwrap();
            assert_eq!(
                calls.load(Ordering::SeqCst),
                12,
                "only the 429 is due; repeated outages back off"
            );
            server.abort();
            f.close().await;
        })
        .await;
}
