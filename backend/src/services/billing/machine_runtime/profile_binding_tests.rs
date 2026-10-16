use super::*;

async fn insert_variant(f: &Fixture, fields: Document) -> String {
    let mut row = bson::to_document(&f.rate).unwrap();
    let id = Uuid::new_v4().to_string();
    row.insert("_id", &id);
    row.insert(
        "lago_metric_code",
        format!("remote_machine_running_seconds_{}", id.replace('-', "")),
    );
    row.insert(
        "created_at",
        bson::DateTime::from_chrono(Utc::now() + Duration::seconds(1)),
    );
    row.extend(fields);
    f.db.collection::<Document>(RATE_COLLECTION)
        .insert_one(row)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn staging_binds_approved_profile_and_price_before_insert() {
    let f = fixture("runtime_profile_staging").await;
    let count =
        f.db.collection::<Document>(RATE_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap();
    for profile in ["", "shell-files-v1", "test-only-foreign-profile"] {
        assert!(
            f.billing
                .stage_machine_runtime_rate(profile, "0.000040".parse().unwrap())
                .await
                .is_err()
        );
    }
    for price in ["0", "-0.000040", "0.000080", "0.000040000001"] {
        assert!(
            f.billing
                .stage_machine_runtime_rate("remote-machine", price.parse().unwrap())
                .await
                .is_err()
        );
    }
    assert_eq!(
        f.db.collection::<Document>(RATE_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        count
    );
    let staged = f
        .billing
        .stage_machine_runtime_rate("remote-machine", "0.000040".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(staged.profile_version, "remote-machine");
    assert_ne!(staged.id, f.rate.id);
    assert_ne!(staged.lago_metric_code, f.rate.lago_metric_code);
    assert_eq!(staged.sync_status, PricingSyncStatus::Pending);
    assert_eq!(staged.credits_per_second, f.rate.credits_per_second);
    assert_eq!(f.billing.runtime_rate(&f.rate.id).await.unwrap(), f.rate);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn selector_excludes_foreign_invalid_unsynced_retired_and_other_plan_rates() {
    let f = fixture("runtime_profile_selector").await;
    for fields in [
        doc! {"profile_version":"test-only-foreign-profile","credits_per_second":Credits::from_whole(9)},
        doc! {"profile_version":bson::Bson::Null},
        doc! {"sync_status":"pending"},
        doc! {"sync_status":"failed"},
        doc! {"verified_at":bson::Bson::Null},
        doc! {"retired":true},
        doc! {"plan_code":"other-plan"},
        doc! {"credits_per_second":Credits::ZERO},
        doc! {"metric":"requests"},
        doc! {"lago_metric_code":"malformed-metric-code"},
    ] {
        insert_variant(&f, fields).await;
    }
    let selected = f
        .billing
        .latest_machine_runtime_rate("remote-machine")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.id, f.rate.id);
    let quote = f
        .billing
        .quote_machine_runtime(&f.owner, f.terms.clone(), &selected.id)
        .await
        .unwrap();
    let view = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", None)
        .await
        .unwrap();
    assert!(view.ready);
    assert_eq!(view.profile_version, quote.terms.profile_version);
    assert_eq!(
        view.rate_version.as_deref(),
        Some(quote.rate_version.as_str())
    );
    assert_eq!(view.credits_per_second, Some(quote.credits_per_second));
    assert_eq!(view.funds.unwrap().estimated_seconds, "250000");
    f.db.collection::<Document>(RATE_COLLECTION)
        .delete_one(doc! {"_id":&f.rate.id})
        .await
        .unwrap();
    assert!(
        f.billing
            .latest_machine_runtime_rate("remote-machine")
            .await
            .unwrap()
            .is_none()
    );
    let missing = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", None)
        .await
        .unwrap();
    assert!(!missing.ready);
    assert_eq!(missing.reason, Some("profile_pricing_unavailable"));
    assert!(missing.rate_version.is_none());
    assert!(missing.funds.is_none());
    assert!(
        f.billing
            .latest_machine_runtime_rate("test-only-foreign-profile")
            .await
            .is_err()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn pinned_preflight_retains_quote_profile_exact_price_and_retired_rate() {
    let f = fixture("runtime_profile_pinned").await;
    let quote = quote(&f).await;
    let new_id = insert_variant(
        &f,
        doc! {"credits_per_second":"0.000080".parse::<Credits>().unwrap()},
    )
    .await;
    f.billing.sync_machine_runtime_rate(&new_id).await.unwrap();
    let latest = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", None)
        .await
        .unwrap();
    assert_eq!(latest.rate_version.as_deref(), Some(new_id.as_str()));
    assert_eq!(latest.funds.unwrap().estimated_seconds, "125000");
    f.billing
        .retire_machine_runtime_rate(&f.rate.id)
        .await
        .unwrap();
    let pinned = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", Some(&quote))
        .await
        .unwrap();
    assert!(pinned.ready);
    assert_eq!(pinned.rate_version.as_deref(), Some(f.rate.id.as_str()));
    assert_eq!(pinned.credits_per_second, Some(quote.credits_per_second));
    assert_eq!(pinned.funds.unwrap().estimated_seconds, "250000");
    let mut changed = quote.clone();
    changed.credits_per_second = "0.000080".parse().unwrap();
    let rejected = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", Some(&changed))
        .await
        .unwrap();
    assert!(!rejected.ready);
    assert!(rejected.funds.is_none());
    changed = quote.clone();
    changed.terms.profile_version = "test-only-foreign-profile".into();
    assert!(
        f.billing
            .machine_runtime_preflight(&f.owner, "remote-machine", Some(&changed))
            .await
            .is_err()
    );
    changed = quote;
    changed.owner_id = Uuid::new_v4().to_string();
    assert!(
        f.billing
            .machine_runtime_preflight(&f.owner, "remote-machine", Some(&changed))
            .await
            .is_err()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn missing_only_profile_compatibility_preserves_rate_and_embedded_snapshot() {
    let f = fixture("runtime_profile_legacy").await;
    let original = f.rate.clone();
    f.db.collection::<Document>(RATE_COLLECTION)
        .update_one(
            doc! {"_id":&f.rate.id},
            doc! {"$unset":{"profile_version":""}},
        )
        .await
        .unwrap();
    assert_eq!(f.billing.runtime_rate(&f.rate.id).await.unwrap(), original);
    assert_eq!(
        f.billing
            .latest_machine_runtime_rate("remote-machine")
            .await
            .unwrap()
            .unwrap(),
        original
    );
    assert!(
        !f.db
            .collection::<Document>(RATE_COLLECTION)
            .find_one(doc! {"_id":&f.rate.id})
            .await
            .unwrap()
            .unwrap()
            .contains_key("profile_version")
    );
    let now = Utc::now();
    let (funded, _, _) = start(
        &f,
        &Uuid::new_v4().to_string(),
        now,
        now + Duration::seconds(30),
    )
    .await;
    ready(&f, &funded.run_id, 1, Utc::now()).await;
    f.db.collection::<Document>(RUN_COLLECTION)
        .update_one(
            doc! {"_id":&funded.run_id},
            doc! {"$unset":{"rate.profile_version":""}},
        )
        .await
        .unwrap();
    let restored = run(&f, &funded.run_id).await;
    assert_eq!(restored.rate, original);
    assert_eq!(restored.terms.profile_version, "remote-machine");
    let mut snapshot = bson::to_document(&restored).unwrap();
    snapshot
        .get_document_mut("rate")
        .unwrap()
        .insert("profile_version", bson::Bson::Null);
    assert!(bson::from_document::<MachineRuntimeRun>(snapshot).is_err());
    let mut rate_doc = bson::to_document(&original).unwrap();
    rate_doc.insert("profile_version", bson::Bson::Null);
    assert!(bson::from_document::<MachineRuntimeRate>(rate_doc).is_err());
    let mut unknown = original;
    unknown.profile_version = "unknown-not-current-default".into();
    assert!(prices::validate_rate(&unknown).is_err());
    let next = f
        .billing
        .prepare_machine_next_window(
            &f.owner,
            &funded.run_id,
            &Uuid::new_v4().to_string(),
            now + Duration::seconds(30),
            now + Duration::seconds(60),
        )
        .await
        .unwrap();
    let renewed = tx!(
        &f,
        session,
        f.billing
            .reserve_machine_next_window_in_session(&mut session, &next)
    )
    .unwrap();
    assert_eq!(renewed.run_id, funded.run_id);
    assert_eq!(renewed.rate_version, f.rate.id);
    assert_eq!(
        run(&f, &funded.run_id).await.rate.credits_per_second,
        f.rate.credits_per_second
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn quote_and_live_rate_profile_mismatches_abort_before_shared_funds() {
    let f = fixture("runtime_profile_admission").await;
    let quote = quote(&f).await;
    let before = bson::to_document(&wallet(&f).await).unwrap();
    let now = Utc::now();
    let interval = Uuid::new_v4().to_string();
    f.db.collection::<Document>(QUOTE_COLLECTION)
        .update_one(
            doc! {"_id":&quote.id},
            doc! {"$set":{"terms.profile_version":"shell-files-v1"}},
        )
        .await
        .unwrap();
    assert!(
        f.billing
            .prepare_machine_start(
                &f.owner,
                &quote.id,
                &interval,
                now,
                now + Duration::seconds(30)
            )
            .await
            .is_err()
    );
    f.db.collection::<Document>(QUOTE_COLLECTION)
        .replace_one(doc! {"_id":&quote.id}, bson::to_document(&quote).unwrap())
        .await
        .unwrap();
    let prepared = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &quote.id,
            &interval,
            now,
            now + Duration::seconds(30),
        )
        .await
        .unwrap();
    f.db.collection::<Document>(RATE_COLLECTION)
        .update_one(
            doc! {"_id":&f.rate.id},
            doc! {"$set":{"profile_version":"test-only-foreign-profile"}},
        )
        .await
        .unwrap();
    assert!(
        accept(
            &f,
            &prepared,
            &Uuid::new_v4().to_string(),
            &Uuid::new_v4().to_string(),
            1
        )
        .await
        .is_err()
    );
    assert_eq!(bson::to_document(&wallet(&f).await).unwrap(), before);
    for collection in [RUN_COLLECTION, INTERVAL_COLLECTION, USAGE] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn embedded_profile_mismatch_fences_renewal_and_accepted_replay() {
    let f = fixture("runtime_profile_run_fence").await;
    let now = Utc::now();
    let node = Uuid::new_v4().to_string();
    let quote = quote(&f).await;
    let interval = Uuid::new_v4().to_string();
    let operation = Uuid::new_v4().to_string();
    let prepared = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &quote.id,
            &interval,
            now,
            now + Duration::seconds(30),
        )
        .await
        .unwrap();
    let funded = accept(&f, &prepared, &node, &operation, 1).await.unwrap();
    ready(&f, &funded.run_id, 1, Utc::now()).await;
    let next = f
        .billing
        .prepare_machine_next_window(
            &f.owner,
            &funded.run_id,
            &Uuid::new_v4().to_string(),
            now + Duration::seconds(30),
            now + Duration::seconds(60),
        )
        .await
        .unwrap();
    let before = bson::to_document(&wallet(&f).await).unwrap();
    f.db.collection::<Document>(RUN_COLLECTION)
        .update_one(
            doc! {"_id":&funded.run_id},
            doc! {"$set":{"rate.profile_version":"test-only-foreign-profile"}},
        )
        .await
        .unwrap();
    assert!(
        f.billing
            .prepare_machine_next_window(
                &f.owner,
                &funded.run_id,
                &Uuid::new_v4().to_string(),
                now + Duration::seconds(30),
                now + Duration::seconds(60)
            )
            .await
            .is_err()
    );
    assert!(
        tx!(
            &f,
            session,
            f.billing
                .reserve_machine_next_window_in_session(&mut session, &next)
        )
        .is_err()
    );
    assert!(
        f.billing
            .replay_machine_start(
                AcceptRuntimeQuote {
                    owner_id: &f.owner,
                    node_id: &node,
                    operation_id: &operation,
                    runtime_generation: 1,
                    terms: &f.terms
                },
                &quote.id,
                &interval,
                now,
                now + Duration::seconds(30)
            )
            .await
            .is_err()
    );
    assert_eq!(bson::to_document(&wallet(&f).await).unwrap(), before);
    f.db.drop().await.unwrap();
}
