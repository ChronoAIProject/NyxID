use super::*;

#[tokio::test]
async fn usage_comparison_round2_ten_minute_gap_keeps_durable_coverage() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            let before = coverage::refresh(&f.db).await.unwrap();
            TEST_NOW.with(|clock| {
                clock.set(bson::DateTime::from_millis(
                    initial_now().timestamp_millis() + 600_000,
                ))
            });
            let after = coverage::refresh(&f.db).await.unwrap();
            assert_eq!(after.generation, before.generation);
            assert_eq!(after.continuous_since, before.continuous_since);
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round2_unreadable_evidence_leaves_coverage_unchanged() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            let before =
                f.db.collection::<Document>(CONTROL_COLLECTION)
                    .find_one(doc! {"_id":COVERAGE_ID})
                    .await
                    .unwrap()
                    .unwrap();
            f.db.collection::<Document>(
                crate::models::security_capability_activation::COLLECTION_NAME,
            )
            .update_one(
                doc! {"capability":coverage::READINESS},
                doc! {"$set":{"active_since":"unreadable"}},
            )
            .await
            .unwrap();
            f.target("sub", 0).await;
            assert!(f.pass().await.1.is_err());
            assert!(
                f.db.collection::<Document>(COLLECTION_NAME)
                    .find_one(doc! {"_id":"sub"})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_document("comparison")
                    .is_err()
            );
            let after =
                f.db.collection::<Document>(CONTROL_COLLECTION)
                    .find_one(doc! {"_id":COVERAGE_ID})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(after, before);
            assert!(f.provider.calls.lock().unwrap().is_empty());
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round2_warming_reads_only_once_in_a_period() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 0).await;
            f.db.collection::<Document>(CONTROL_COLLECTION)
                .update_one(
                    doc! {"_id":COVERAGE_ID},
                    doc! {"$set":{"continuous_since":initial_now()}},
                )
                .await
                .unwrap();
            f.pass().await.1.unwrap();
            f.advance(300).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(), 1);
            let line=diagnostics(&f.db).await.unwrap().into_iter().find(|d|d.code=="billing_usage_coverage").unwrap();
            assert_eq!(line.detail.trim(),"Usage comparison starts with the next full billing period. Capture has been continuous since 15 Nov 2026 12:00 UTC. 1 subscription is waiting for that full billing period.");
            let seconds=at("2026-12-01T00:00:00Z").to_chrono().timestamp()-TEST_NOW.with(|c|c.get().to_chrono().timestamp());
            f.advance(seconds).await;
            let mut next=usage("0");
            next["customer_usage"]["from_datetime"]=serde_json::json!("2026-12-01T00:00:00Z");
            next["customer_usage"]["to_datetime"]=serde_json::json!("2026-12-31T23:59:59Z");
            f.provider.replies.lock().unwrap().insert("sub".into(),Reply::Usage(next));
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(),2);
            assert_eq!(f.observation("sub").await.status,"equal");
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round2_discovery_does_not_rewrite_100_present_targets() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let writes = Arc::new(AtomicUsize::new(0));
    let seen = writes.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "usage_discovery_cost",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command.get_str("update").ok() == Some(COLLECTION_NAME)
            {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        }),
    )
    .await;
    for n in 0..100 {
        db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME).insert_one(doc!{
            "_id":format!("wallet-{n:03}"),"owner_id":format!("owner-{n:03}"),"lago_customer_id":format!("customer-{n:03}"),"lago_subscription_id":format!("sub-{n:03}")
        }).await.unwrap();
        db.collection::<Document>(COLLECTION_NAME).insert_one(doc! {
            "_id":format!("target-{n:03}"),"wallet_id":format!("wallet-{n:03}"),"owner_id":format!("owner-{n:03}"),"customer_id":format!("customer-{n:03}"),"subscription_id":format!("sub-{n:03}")
        }).await.unwrap();
    }
    writes.store(0, Ordering::SeqCst);
    schedule::discover(&db).await.unwrap();
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_comparison_round2_retained_unsupported_evidence_and_horizon_reset_once() {
    TEST_NOW.scope(std::cell::Cell::new(initial_now()), async {
        let f=Fixture::new().await;
        f.target("sub",0).await;
        f.pass().await.1.unwrap();
        assert_eq!(f.observation("sub").await.status,"pending");
        let before=coverage::refresh(&f.db).await.unwrap();
        f.db.collection::<Document>(crate::models::security_replica::COLLECTION_NAME).insert_one(doc! {
            "_id":"old-generation","protocol":1,"capabilities":[],
            "observed_at":bson::DateTime::from_millis(initial_now().timestamp_millis()+60_000)
        }).await.unwrap();
        f.advance(600).await;
        f.pass().await.1.unwrap();
        assert_eq!(f.observation("sub").await.status,"coverage","confirmation cannot span an unsupported generation");
        let after=coverage::refresh(&f.db).await.unwrap();
        assert_ne!(before.generation,after.generation);
        assert_eq!(after.continuous_since,Some(initial_now().to_chrono()+chrono::Duration::minutes(10)));
        assert_eq!(coverage::refresh(&f.db).await.unwrap().generation,after.generation,"retained dead witness resets once");
        TEST_NOW.with(|clock|clock.set(bson::DateTime::from_millis(clock.get().timestamp_millis()+coverage::EVIDENCE_HORIZON_MILLIS)));
        let horizon=coverage::refresh(&f.db).await.unwrap();
        assert_ne!(horizon.generation,after.generation);
        assert_eq!(horizon.continuous_since,Some(horizon.observed_at));
        assert_eq!(coverage::refresh(&f.db).await.unwrap().generation,horizon.generation);
        f.close().await;
    }).await;
}

#[tokio::test]
async fn usage_comparison_round2_local_wait_equal_due_and_retirement() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 0).await;
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Usage(usage("0")));
            f.commands.lock().unwrap().clear();
            f.pass().await.1.unwrap();
            let commands = f.commands.lock().unwrap().clone();
            println!(
                "USAGE_COMPARISON_FIRST_READ commands={} names={:?}",
                commands.len() - 2,
                commands
                    .iter()
                    .map(|(n, d)| (n, d.get_str(n)))
                    .collect::<Vec<_>>()
            );
            assert_eq!(f.observation("sub").await.status, "equal");
            f.advance(6 * 3600 - 1).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(), 1);
            f.advance(1).await;
            f.db.collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":"sub"},
                    doc! {"$set":{"last_acknowledged_at":TEST_NOW.with(|clock|clock.get())}},
                )
                .await
                .unwrap();
            f.commands.lock().unwrap().clear();
            f.pass().await.1.unwrap();
            println!(
                "USAGE_COMPARISON_LOCAL_QUIET commands={}",
                f.commands.lock().unwrap().len() - 2
            );
            assert_eq!(
                f.provider.calls.lock().unwrap().len(),
                1,
                "known busy period needs no read"
            );
            assert_eq!(
                f.observation("sub").await.reason,
                "acknowledgements_not_quiet"
            );
            f.advance(600).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(), 2);
            f.receipt("sub", 1, initial_now().to_chrono().timestamp(), "1")
                .await;
            f.db.collection::<Document>(crate::models::billing_usage_delivery::COLLECTION_NAME)
                .update_one(
                    doc! {"transaction_id":"sub:1"},
                    doc! {"$set":{"state":"prepared","acknowledgement_sequence":Bson::Null}},
                )
                .await
                .unwrap();
            f.db.collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":"sub"},
                    doc! {"$set":{"next_visit_at":Bson::Null}},
                )
                .await
                .unwrap();
            f.pass().await.1.unwrap();
            assert_eq!(
                f.provider.calls.lock().unwrap().len(),
                2,
                "known prepared period needs no read"
            );
            assert_eq!(
                f.observation("sub").await.reason,
                "unresolved_deliveries_in_period"
            );
            f.db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                .delete_one(doc! {"_id":"wallet-sub"})
                .await
                .unwrap();
            f.advance(6 * 3600).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.observation("sub").await.status, "retired");
            f.advance(86400).await;
            f.pass().await.1.unwrap();
            assert_eq!(
                f.provider.calls.lock().unwrap().len(),
                2,
                "retired binding costs no reads"
            );
            assert!(diagnostics(&f.db).await.unwrap().is_empty());
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round2_duplicate_origin_is_sticky_only_in_its_period() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            for created in [
                Some(at("2026-10-02T00:00:00Z")),
                Some(at("2026-09-01T00:00:00Z")),
                None,
            ] {
                TEST_NOW.with(|clock| clock.set(initial_now()));
                let f = Fixture::new().await;
                f.target("sub", 1).await;
                f.receipt("sub", 1, initial_now().to_chrono().timestamp(), "1")
                    .await;
                f.db.collection::<Document>(crate::models::billing_usage_delivery::COLLECTION_NAME)
                    .update_one(
                        doc! {"transaction_id":"sub:1"},
                        doc! {"$set":{"duplicate_acknowledgement":true,"usage_created_at":created}},
                    )
                    .await
                    .unwrap();
                f.pass().await.1.unwrap();
                let known = created.is_some_and(|at| at > super::at("2026-10-01T00:00:00Z"));
                assert_eq!(
                    f.observation("sub").await.status,
                    if known { "equal" } else { "coverage" }
                );
                let target =
                    f.db.collection::<BillingUsageComparison>(COLLECTION_NAME)
                        .find_one(doc! {"_id":"sub"})
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(
                    target.projection.as_ref().unwrap().unverified_origin,
                    !known
                );
                // Force a second local fold to prove unknown origin remains on incremental progress.
                f.db.collection::<Document>(COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":"sub"},
                        doc! {"$set":{"next_visit_at":Bson::Null}},
                    )
                    .await
                    .unwrap();
                f.advance(600).await;
                f.pass().await.1.unwrap();
                if !known {
                    assert_eq!(
                        f.observation("sub").await.reason,
                        "duplicate_delivery_origin_unverified"
                    );
                    assert_eq!(f.provider.calls.lock().unwrap().len(), 1);
                }
                f.advance(20 * 86400).await;
                let mut next = usage("0");
                next["customer_usage"]["from_datetime"] = serde_json::json!("2026-12-01T00:00:00Z");
                next["customer_usage"]["to_datetime"] = serde_json::json!("2026-12-31T23:59:59Z");
                f.provider
                    .replies
                    .lock()
                    .unwrap()
                    .insert("sub".into(), Reply::Usage(next));
                f.pass().await.1.unwrap();
                assert_eq!(
                    f.observation("sub").await.status,
                    "equal",
                    "old duplicate is outside the new period"
                );
                f.close().await;
            }
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round2_clean_pass_cost_is_bounded_and_provider_reads_overlap() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.commands.lock().unwrap().clear();
            f.pass().await.1.unwrap();
            println!(
                "USAGE_COMPARISON_CLEAN wallets=0 commands={}",
                f.commands.lock().unwrap().len() - 2
            );
            for n in 0..100 {
                f.target(&format!("sub-{n:03}"), 0).await;
            }
            f.db.collection::<Document>(COLLECTION_NAME)
                .update_many(
                    doc! {},
                    doc! {"$set":{"next_visit_at":at("2026-12-01T00:00:00Z")}},
                )
                .await
                .unwrap();
            schedule::discover(&f.db).await.unwrap();
            f.commands.lock().unwrap().clear();
            f.pass().await.1.unwrap();
            let commands = f.commands.lock().unwrap().clone();
            println!(
                "USAGE_COMPARISON_CLEAN wallets=100 commands={} names={:?}",
                commands.len() - 2,
                commands
                    .iter()
                    .map(|(n, d)| (n, d.get_str(n)))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                commands.len() - 2,
                10,
                "one coverage check, four discovery reads, due and legacy-reason reads"
            );
            assert!(
                !commands.iter().any(
                    |(n, d)| n == "update" && d.get_str("update").ok() == Some(COLLECTION_NAME)
                )
            );
            for n in 0..4 {
                f.db.collection::<Document>(COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":format!("sub-{n:03}")},
                        doc! {"$set":{"next_visit_at":Bson::Null}},
                    )
                    .await
                    .unwrap();
            }
            *f.provider.barrier.lock().unwrap() = Some(Arc::new(tokio::sync::Barrier::new(4)));
            f.commands.lock().unwrap().clear();
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.peak.load(Ordering::SeqCst), 4);
            let commands = f.commands.lock().unwrap().clone();
            assert_eq!(
                commands
                    .iter()
                    .filter(|(name, command)| {
                        name == "update" && command.get_str("update").ok() == Some(COLLECTION_NAME)
                    })
                    .count(),
                4,
                "one atomic update per target"
            );
            assert!(
                !commands
                    .iter()
                    .any(|(_, command)| command.contains_key("autocommit"))
            );
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round2_findings_backoff_and_human_diagnostics() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            for id in ["missing", "reject", "error"] {
                f.target(id, 0).await;
            }
            f.provider.replies.lock().unwrap().insert(
                "missing".into(),
                Reply::Usage(
                    serde_json::json!({"usage_comparison_finding":"subscription_unavailable"}),
                ),
            );
            f.provider.replies.lock().unwrap().insert(
                "reject".into(),
                Reply::Usage(
                    serde_json::json!({"usage_comparison_finding":"provider_request_rejected"}),
                ),
            );
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("error".into(), Reply::Error);
            assert!(f.pass().await.1.is_err());
            assert_eq!(
                f.observation("missing").await.reason,
                "subscription_unavailable"
            );
            assert_eq!(
                f.observation("reject").await.reason,
                "provider_request_rejected"
            );
            let ds = diagnostics(&f.db).await.unwrap();
            let text = &ds
                .iter()
                .find(|d| d.code == "billing_usage_unsupported")
                .unwrap()
                .detail;
            assert!(text.contains("the provider no longer exposes the subscription"));
            assert!(text.contains("the provider rejected the usage request"));
            assert!(!text.contains("states="));
            assert!(ds.iter().all(|d| d.detail.chars().count() <= 4096));
            f.advance(300).await;
            assert!(f.pass().await.1.is_err());
            assert_eq!(f.provider.calls.lock().unwrap().len(), 4);
            f.advance(300).await;
            f.pass().await.1.unwrap();
            assert_eq!(
                f.provider.calls.lock().unwrap().len(),
                4,
                "second outage backs off to ten minutes"
            );
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round2_retired_shape_finding_is_due_without_bypassing_cap() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            let coverage = coverage::refresh(&f.db).await.unwrap();
            for n in 0..27 {
                let id = format!("old-shape-{n:02}");
                f.target(&id, 0).await;
                let target =
                    f.db.collection::<BillingUsageComparison>(COLLECTION_NAME)
                        .find_one(doc! {"_id":&id})
                        .await
                        .unwrap()
                        .unwrap();
                let reason = if n == 25 {
                    "duplicate_metric"
                } else {
                    "unsupported_usage_shape"
                };
                let observation = observation::waiting(
                    &target,
                    &coverage,
                    initial_now().to_chrono(),
                    "old-pass",
                    "unsupported",
                    reason,
                );
                f.db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&id},
                doc! {"$set":{
                    "comparison":bson::to_bson(&observation).unwrap(),
                    "next_visit_at":at("2026-11-16T12:00:00Z"),
                    "retired_at":if n == 26 { Bson::DateTime(initial_now()) } else { Bson::Null }
                }}).await.unwrap();
                if n == 26 {
                    f.db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                        .delete_one(doc! {"_id":format!("wallet-{id}")})
                        .await
                        .unwrap();
                }
                let mut real = usage("0.0");
                real["customer_usage"]["charges_usage"][0]["filters"] = serde_json::json!([]);
                real["customer_usage"]["charges_usage"][0]["grouped_usage"] = serde_json::json!([]);
                f.provider
                    .replies
                    .lock()
                    .unwrap()
                    .insert(id, Reply::Usage(real));
            }
            assert_eq!(
                schedule::select(&f.db, initial_now().to_chrono())
                    .await
                    .unwrap()
                    .len(),
                20
            );
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(), 20);
            for n in 0..20 {
                assert_eq!(
                    f.observation(&format!("old-shape-{n:02}")).await.status,
                    "equal"
                );
            }
            assert_eq!(
                schedule::select(&f.db, initial_now().to_chrono())
                    .await
                    .unwrap()
                    .len(),
                5
            );
            f.pass().await.1.unwrap();
            assert_eq!(
                f.provider.calls.lock().unwrap().len(),
                20,
                "rolling budget still applies"
            );
            f.advance(300).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(), 25);
            for n in 20..25 {
                assert_eq!(
                    f.observation(&format!("old-shape-{n:02}")).await.status,
                    "equal"
                );
            }
            assert!(
                schedule::select(&f.db, TEST_NOW.with(|now| now.get().to_chrono()))
                    .await
                    .unwrap()
                    .is_empty()
            );
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_round2_parser_findings_are_stored_and_explained_without_payloads() {
    TEST_NOW.scope(std::cell::Cell::new(initial_now()), async {
        let f = Fixture::new().await;
        let cases = [
            ("missing_customer_usage", "the provider omitted customer_usage"),
            ("missing_charges_usage", "the provider omitted charges_usage or returned it in a non-array form"),
            ("charge_not_object", "a charges_usage entry is not an object"),
            ("too_many_charges", "the provider returned more than 256 charges"),
            ("non_sum_aggregation:count_agg", "a delivered metric has non-sum provider aggregation 'count_agg'"),
            ("non_sum_aggregation:other", "a delivered metric has non-sum provider aggregation 'other'"),
            ("breakdown_mismatch", "a provider usage breakdown does not add up to its charge total"),
            ("duplicate_metric", "the provider returned the same metric more than once"),
            ("missing_metric_code", "the provider omitted a metric code"),
            ("unsupported_provider_units", "the quantities cannot be represented exactly"),
            ("invalid_provider_period", "the provider period is invalid"),
        ];
        for (n, (reason, _)) in cases.iter().enumerate() {
            let id = format!("finding-{n:02}");
            let non_sum = reason.starts_with("non_sum_aggregation:");
            f.target(&id, i64::from(non_sum)).await;
            if non_sum {
                f.receipt(&id, 1, initial_now().to_chrono().timestamp() - 3600, "1").await;
            }
            let mut raw = usage("1");
            raw["customer_usage"]["charges_usage"][0]["charge"] = serde_json::json!({"lago_id":"fixture-payload-never-store"});
            match *reason {
                "missing_customer_usage" => { raw.as_object_mut().unwrap().remove("customer_usage"); }
                "missing_charges_usage" => { raw["customer_usage"].as_object_mut().unwrap().remove("charges_usage"); }
                "charge_not_object" => raw["customer_usage"]["charges_usage"][0] = serde_json::json!(17),
                "too_many_charges" => raw["customer_usage"]["charges_usage"] = serde_json::json!(vec![raw["customer_usage"]["charges_usage"][0].clone(); 257]),
                "non_sum_aggregation:count_agg" => raw["customer_usage"]["charges_usage"][0]["billable_metric"]["aggregation_type"] = serde_json::json!("count_agg"),
                "non_sum_aggregation:other" => raw["customer_usage"]["charges_usage"][0]["billable_metric"]["aggregation_type"] = serde_json::json!("fixture-payload-never-store"),
                "breakdown_mismatch" => raw["customer_usage"]["charges_usage"][0]["filters"] = serde_json::json!([{"units":"2"}]),
                "duplicate_metric" => raw["customer_usage"]["charges_usage"] = serde_json::json!(vec![raw["customer_usage"]["charges_usage"][0].clone(); 2]),
                "missing_metric_code" => raw["customer_usage"]["charges_usage"][0]["billable_metric"]["code"] = serde_json::Value::Null,
                "unsupported_provider_units" => raw["customer_usage"]["charges_usage"][0]["units"] = serde_json::json!("fixture-payload-never-store"),
                "invalid_provider_period" => raw["customer_usage"]["from_datetime"] = serde_json::json!("fixture-payload-never-store"),
                _ => unreachable!(),
            }
            f.provider.replies.lock().unwrap().insert(id, Reply::Usage(raw));
        }
        let (stats, result) = f.pass().await;
        result.unwrap();
        assert_eq!(stats.drift_alerts, 0);
        let items = diagnostics(&f.db).await.unwrap();
        let detail = &items.iter().find(|item| item.code == "billing_usage_unsupported").unwrap().detail;
        for (n, (reason, sentence)) in cases.iter().enumerate() {
            let row = f.db.collection::<Document>(COLLECTION_NAME).find_one(doc! {"_id":format!("finding-{n:02}")}).await.unwrap().unwrap();
            let observation = row.get_document("comparison").unwrap();
            assert_eq!(observation.get_str("status").unwrap(), "unsupported");
            assert_eq!(observation.get_str("reason").unwrap(), *reason);
            for key in ["local", "remote", "differences"] {
                assert!(observation.get_document(key).unwrap().is_empty());
            }
            assert!(!serde_json::to_string(&row).unwrap().contains("fixture-payload-never-store"));
            assert!(detail.contains(&format!("1 subscription cannot be compared: {sentence}.")), "{detail}");
        }
        f.close().await;
    }).await;
}
