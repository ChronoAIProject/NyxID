use super::*;

fn scheduled_row(id: &str, next: bson::DateTime) -> Document {
    doc! {"_id":id,"owner_id":id,"customer_id":id,"subscription_id":id,
    "retired_at":Bson::Null,"next_visit_at":next}
}

async fn selection_stats(f: &Fixture, expected: usize, legacy: bool) -> Vec<Document> {
    f.commands.lock().unwrap().clear();
    let selected = schedule::select(&f.db, initial_now().to_chrono())
        .await
        .unwrap();
    assert_eq!(selected.len(), expected);
    let queries: Vec<_> = f
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, command)| name == "find" && command.get_str("find") == Ok(COLLECTION_NAME))
        .map(|(_, command)| command.clone())
        .collect();
    for (n, command) in queries.iter().enumerate() {
        let query: Document = ["find", "filter", "sort", "limit", "hint"]
            .into_iter()
            .filter_map(|key| command.get(key).map(|v| (key.to_owned(), v.clone())))
            .collect();
        let evidence =
            f.db.run_command(doc! {"explain":query,"verbosity":"executionStats"})
                .await
                .unwrap();
        let stats = evidence.get_document("executionStats").unwrap();
        let value = |key| {
            stats
                .get_i64(key)
                .or_else(|_| stats.get_i32(key).map(i64::from))
                .unwrap()
        };
        let (returned, documents, keys) = (
            value("nReturned"),
            value("totalDocsExamined"),
            value("totalKeysExamined"),
        );
        println!(
            "USAGE_SHAPE_SELECTION legacy={legacy} query={n} returned={returned} documents={documents} keys={keys}"
        );
        assert_eq!(documents, returned, "{stats:?}");
        assert!(keys <= returned, "{stats:?}");
    }
    assert_eq!(queries.len(), 2);
    selected
}

#[tokio::test]
async fn usage_comparison_review_selection_examines_only_due_rows() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            let future = at("2026-11-16T12:00:00Z");
            f.db.collection::<Document>(COLLECTION_NAME)
                .insert_many((0..1000).map(|n| scheduled_row(&format!("future-{n:04}"), future)))
                .await
                .unwrap();
            selection_stats(&f, 0, false).await;
            for n in 0..3 {
                f.db.collection::<Document>(COLLECTION_NAME)
                    .insert_one(scheduled_row(
                        &format!("due-{n}"),
                        at("2026-11-15T11:00:00Z"),
                    ))
                    .await
                    .unwrap();
            }
            selection_stats(&f, 3, false).await;
            f.db.collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":"due-0"},
                    doc! {"$set":{"comparison.reason":"unsupported_usage_shape"}},
                )
                .await
                .unwrap();
            for n in 0..25 {
                let mut row = scheduled_row(&format!("legacy-{n:02}"), future);
                row.insert("comparison", doc! {"reason":"unsupported_usage_shape"});
                f.db.collection::<Document>(COLLECTION_NAME)
                    .insert_one(row)
                    .await
                    .unwrap();
            }
            let selected = selection_stats(&f, 20, true).await;
            assert_eq!(
                selected[..3]
                    .iter()
                    .map(|r| r.get_str("_id").unwrap())
                    .collect::<Vec<_>>(),
                vec!["due-0", "due-1", "due-2"]
            );
            let ids: std::collections::BTreeSet<_> =
                selected.iter().map(|r| r.get_str("_id").unwrap()).collect();
            assert_eq!(
                ids.len(),
                20,
                "a due legacy finding must not be selected twice"
            );
            assert!(
                selected[3..]
                    .iter()
                    .all(|r| r.get_str("_id").unwrap().starts_with("legacy-"))
            );
            f.db.collection::<Document>(COLLECTION_NAME)
                .update_many(
                    doc! {},
                    doc! {"$set":{"next_visit_at":future},"$unset":{"comparison.reason":""}},
                )
                .await
                .unwrap();
            selection_stats(&f, 0, false).await;
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_review_non_sum_only_blocks_delivered_metrics() {
    TEST_NOW.scope(std::cell::Cell::new(initial_now()), async {
        let f = Fixture::new().await;
        for (id, count_delivered, bad_breakdown) in [
            ("unused", false, false),
            ("unused-bad-breakdown", false, true),
            ("delivered", true, false),
            ("outside-period", false, false),
        ] {
            let has_count_receipt = count_delivered || id == "outside-period";
            f.target(id, if has_count_receipt {2} else {1}).await;
            f.receipt(id, 1, initial_now().to_chrono().timestamp() - 3600, "1").await;
            if has_count_receipt {
                let second = if count_delivered { initial_now().to_chrono().timestamp() - 1800 }
                    else { at("2026-10-15T12:00:00Z").to_chrono().timestamp() };
                f.receipt(id, 2, second, "1").await;
                f.db.collection::<Document>(crate::models::billing_usage_delivery::COLLECTION_NAME)
                    .update_one(doc! {"subscription_id":id,"acknowledgement_sequence":2_i64},
                        doc! {"$set":{"code":"provider_count"}}).await.unwrap();
            }
            let mut raw = usage("1");
            raw["customer_usage"]["charges_usage"].as_array_mut().unwrap().push(serde_json::json!({
                "billable_metric":{"code":"provider_count","aggregation_type":"count_agg"},
                "units":"7", "filters":if bad_breakdown {serde_json::json!([{"units":"999"}])} else {serde_json::json!([])},
                "grouped_usage":if bad_breakdown {serde_json::json!([{"units":"invalid"}])} else {serde_json::json!([])}
            }));
            f.provider.replies.lock().unwrap().insert(id.into(), Reply::Usage(raw));
        }
        let (stats, result) = f.pass().await;
        result.unwrap();
        assert_eq!(stats.drift_alerts, 0);
        for id in ["unused", "unused-bad-breakdown", "outside-period"] {
            let observation = f.observation(id).await;
            assert_eq!(observation.status, "equal", "{id}: {observation:?}");
            assert_eq!(observation.local, BTreeMap::from([("requests".into(), "1".into())]));
            assert_eq!(observation.remote, observation.local);
        }
        let finding = f.observation("delivered").await;
        assert_eq!(finding.status, "unsupported");
        assert_eq!(finding.reason, "non_sum_aggregation:count_agg");
        assert!(finding.local.is_empty() && finding.remote.is_empty() && finding.differences.is_empty());
        f.close().await;
    }).await;
}

#[tokio::test]
async fn usage_comparison_followup_missing_legacy_index_preserves_due_work() {
    use tracing::instrument::WithSubscriber;
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            let coverage = coverage::refresh(&f.db).await.unwrap();
            for id in ["due", "legacy"] {
                f.target(id, 1).await;
                f.receipt(id, 1, initial_now().to_chrono().timestamp() - 3600, "1")
                    .await;
            }
            let targets = f.db.collection::<BillingUsageComparison>(COLLECTION_NAME);
            let target = targets
                .find_one(doc! {"_id":"legacy"})
                .await
                .unwrap()
                .unwrap();
            let previous = observation::waiting(
                &target,
                &coverage,
                initial_now().to_chrono(),
                "old-pass",
                "unsupported",
                "unsupported_usage_shape",
            );
            let rows = f.db.collection::<Document>(COLLECTION_NAME);
            rows.update_one(
                doc! {"_id":"legacy"},
                doc! {"$set":{
                    "next_visit_at":at("2026-11-16T12:00:00Z"),
                    "comparison":bson::to_bson(&previous).unwrap()
                }},
            )
            .await
            .unwrap();
            let before = rows.find_one(doc! {"_id":"legacy"}).await.unwrap().unwrap();
            rows.drop_index(super::super::super::usage_delivery::LEGACY_USAGE_SHAPE_INDEX)
                .await
                .unwrap();
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_target(false)
                .with_max_level(tracing::Level::WARN)
                .with_writer(log.reopen().unwrap())
                .finish();
            let (stats, result) = f.pass().with_subscriber(subscriber).await;
            result.unwrap();
            assert_eq!(stats.drift_alerts, 0);
            assert_eq!(f.observation("due").await.status, "equal");
            assert_eq!(
                rows.find_one(doc! {"_id":"legacy"}).await.unwrap().unwrap(),
                before
            );
            assert_eq!(
                f.provider.calls.lock().unwrap().as_slice(),
                &[("customer-due".into(), "due".into())]
            );
            let warning = std::fs::read_to_string(log.path()).unwrap();
            assert_eq!(warning.lines().count(), 1, "{warning}");
            assert_eq!(
                warning.trim(),
                "WARN Legacy usage lookup skipped because its index is unavailable database_code=2"
            );
            println!("USAGE_SHAPE_MISSING_INDEX due=equal legacy=untouched warnings=1 code=2");
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_followup_non_sum_units_do_not_block_unused_metrics() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            for (id, aggregation, units, delivered) in [
                ("unused-count", "count_agg", "invalid", false),
                ("unused-weighted", "weighted_sum_agg", "0.0000001", false),
                ("delivered-count", "count_agg", "invalid", true),
                ("delivered-weighted", "weighted_sum_agg", "0.0000001", true),
            ] {
                f.target(id, if delivered { 2 } else { 1 }).await;
                f.receipt(id, 1, initial_now().to_chrono().timestamp() - 3600, "1")
                    .await;
                if delivered {
                    f.receipt(id, 2, initial_now().to_chrono().timestamp() - 1800, "1")
                        .await;
                    f.db.collection::<Document>(
                        crate::models::billing_usage_delivery::COLLECTION_NAME,
                    )
                    .update_one(
                        doc! {"subscription_id":id,"acknowledgement_sequence":2_i64},
                        doc! {"$set":{"code":"extra"}},
                    )
                    .await
                    .unwrap();
                }
                let mut raw = usage("1");
                raw["customer_usage"]["charges_usage"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({
                        "billable_metric":{"code":"extra","aggregation_type":aggregation},
                        "units":units,"filters":[{"units":"invalid"}],"grouped_usage":"invalid"
                    }));
                f.provider
                    .replies
                    .lock()
                    .unwrap()
                    .insert(id.into(), Reply::Usage(raw));
            }
            let (stats, result) = f.pass().await;
            result.unwrap();
            assert_eq!(stats.drift_alerts, 0);
            for id in ["unused-count", "unused-weighted"] {
                let observed = f.observation(id).await;
                assert_eq!(observed.status, "equal", "{id}: {observed:?}");
                assert_eq!(
                    observed.remote,
                    BTreeMap::from([("requests".into(), "1".into())])
                );
                assert_eq!(observed.local, observed.remote);
            }
            for (id, name) in [
                ("delivered-count", "count_agg"),
                ("delivered-weighted", "weighted_sum_agg"),
            ] {
                let observed = f.observation(id).await;
                assert_eq!(observed.status, "unsupported");
                assert_eq!(observed.reason, format!("non_sum_aggregation:{name}"));
            }
            f.close().await;
        })
        .await;
}
