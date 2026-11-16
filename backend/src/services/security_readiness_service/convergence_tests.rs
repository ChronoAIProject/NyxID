use super::activation_tests::assess_every_ten_seconds;
use super::tests::row;
use super::*;
use crate::services::{
    audit_service::AuditActor, billing::model_tariffs, feature_flag_service as flags,
    platform_model_service as models,
};

fn isolated_process(test: &str) -> bool {
    const CHILD: &str = "NYXID_TEST_READINESS_CONVERGENCE_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return true;
    }
    // These cases need different process-wide runtimes, including no runtime.
    // Isolate them under ordinary cargo test as well as nextest.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("services::security_readiness_service::convergence_tests::{test}"),
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, test)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child {test}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    false
}

fn undeclared_runtime() -> Runtime {
    Runtime::new(
        ReplicaIdentity {
            instance_name: "pod-a".into(),
            generation_id: "generation-a".into(),
            internal_base_url: "http://127.0.0.1:3002".into(),
        },
        RolloutConfig::default(),
        false,
    )
}
async fn expire(db: &Database, generation: &str) {
    db.collection::<SecurityReplica>(COLLECTION_NAME).update_one(doc! {"_id":generation},
        doc! {"$set":{"observed_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(MAX_AGE_SECS+1))}}).await.unwrap();
}
async fn model_fixture(
    db: &Database,
) -> (
    AuditActor,
    crate::models::platform_model::PlatformModel,
    model_tariffs::NewTariff,
) {
    models::ensure_indexes(db).await.unwrap();
    model_tariffs::ensure_indexes(db).await.unwrap();
    crate::services::role_service::seed_system_roles(db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(db)
        .await
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let mut user = crate::test_utils::test_user(&id, crate::models::user::UserType::Person);
    user.role_ids.push(roles.admin);
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(user)
        .await
        .unwrap();
    let actor = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&id));
    let model = models::create(
        db,
        &actor,
        models::NewModel {
            public_id: "nyxid/gpt-test".into(),
            display_name: "Convergence test".into(),
            description: String::new(),
            family: crate::models::platform_model::ModelFamily::Openai,
            capabilities: Default::default(),
        },
    )
    .await
    .unwrap();
    let (mut service, tariff, _) = model_tariffs::tests::fixture();
    service.billing = None;
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&service)
    .await
    .unwrap();
    db.collection::<bson::Document>(crate::models::platform_model::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&model.id},
            doc! {"$set":{"billing_service_id":&service.id}},
        )
        .await
        .unwrap();
    let input = model_tariffs::NewTariff {
        revision: model.revision,
        lane: tariff.lane,
        variant: tariff.variant,
        contract: tariff.contract,
        bounds: tariff.bounds,
        components: tariff.components,
        source: tariff.source,
    };
    (actor, model, input)
}
fn model_update(model: &crate::models::platform_model::PlatformModel) -> models::ModelUpdate {
    models::ModelUpdate {
        expected_revision: model.revision,
        display_name: model.display_name.clone(),
        description: model.description.clone(),
        capabilities: model.capabilities.clone(),
        lifecycle: crate::models::platform_model::Lifecycle::Active,
        enabled: true,
        billing_service_id: None,
    }
}

#[tokio::test]
async fn no_readiness_environment_single_distinct_and_shared_names_converge_and_publish_without_warning_audits()
 {
    if !isolated_process(
        "no_readiness_environment_single_distinct_and_shared_names_converge_and_publish_without_warning_audits",
    ) {
        return;
    }
    let local = undeclared_runtime();
    initialize(local.clone());
    for shape in ["single", "distinct", "shared"] {
        local.reset_activation(None);
        let db = crate::test_utils::connect_transaction_test_database("automatic_readiness").await;
        ensure_indexes(&db).await.unwrap();
        publish_observation(&db, &local, CAPABILITIES)
            .await
            .unwrap();
        let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
        if shape != "single" {
            let mut sibling = row(&local, Utc::now());
            sibling.id = "generation-b".into();
            sibling.generation_id = sibling.id.clone();
            if shape == "distinct" {
                sibling.instance_name = "pod-b".into();
            }
            replicas.insert_one(sibling).await.unwrap();
        }
        let (actor, model, mut tariff) = model_fixture(&db).await;
        assert!(matches!(
            models::update(&db, &actor, &model.id, model_update(&model)).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        assert!(matches!(
            model_tariffs::create(&db, &actor, &model.id, tariff.clone()).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        assert!(matches!(
            require_flag_enable(&db, models::FLAG).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        let start = Instant::now();
        record_activations(&db, &local, CAPABILITIES, start)
            .await
            .unwrap();
        assess_every_ten_seconds(
            &db,
            &local,
            CAPABILITIES,
            start,
            start + ACTIVATION_SETTLE_WINDOW - std::time::Duration::from_secs(1),
        )
        .await;
        for capability in CAPABILITIES {
            assert!(
                !serving_report(&db, capability).await.unwrap().serving_ready,
                "{shape}/{capability}"
            );
        }
        record_activations(&db, &local, CAPABILITIES, start + ACTIVATION_SETTLE_WINDOW)
            .await
            .unwrap();
        for capability in CAPABILITIES {
            let state = serving_report(&db, capability).await.unwrap();
            if *capability == PLATFORM_POLICY {
                // This independent opt-in remains disabled, even on a supported fleet.
                assert!(!state.serving_ready && !state.activated);
            } else {
                assert!(
                    !state.ready && state.activated && state.serving_ready,
                    "{shape}/{capability}"
                );
                assert!(
                    state
                        .roster_warnings
                        .contains(&"old_replicas_not_declared_drained".into())
                );
                require_serving(&db, capability).await.unwrap();
            }
        }
        assert!(
            flags::set_platform_override(
                &db,
                models::FLAG,
                &flags::FlagTarget::Global,
                true,
                &actor.user_id
            )
            .await
            .unwrap()
            .enabled
        );
        let published = models::update(&db, &actor, &model.id, model_update(&model))
            .await
            .unwrap();
        // The model update deliberately cleared its optional billing binding;
        // attach the logical offering for tariff publication's independent fence.
        db.collection::<bson::Document>(crate::models::platform_model::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&model.id},
                doc! {"$set":{"billing_service_id":"service"}},
            )
            .await
            .unwrap();
        tariff.revision = published.revision;
        model_tariffs::create(&db, &actor, &model.id, tariff)
            .await
            .unwrap();
        let before = rollout_state(&db, CAPABILITIES).await.unwrap().1;
        expire(&db, "generation-b").await;
        assert!(
            serving_report(&db, models::REGISTRY_READINESS)
                .await
                .unwrap()
                .serving_ready
        );
        let mut restarted = Runtime::new(local.identity.clone(), RolloutConfig::default(), false);
        restarted.identity.generation_id = "generation-restarted".into();
        publish_observation(&db, &restarted, CAPABILITIES)
            .await
            .unwrap();
        expire(&db, &local.identity.generation_id).await;
        assert!(
            serving_report_for(&db, &restarted, models::REGISTRY_READINESS)
                .await
                .unwrap()
                .serving_ready
        );
        let mut third = row(&restarted, Utc::now());
        third.id = "generation-c".into();
        third.generation_id = third.id.clone();
        third.instance_name = "pod-c".into();
        replicas.insert_one(third).await.unwrap();
        assert!(
            serving_report_for(&db, &restarted, models::REGISTRY_READINESS)
                .await
                .unwrap()
                .serving_ready
        );
        assert_eq!(rollout_state(&db, CAPABILITIES).await.unwrap().1, before);
        assert_eq!(
            db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type":diagnostics::EVENT})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn no_environment_rollout_waits_for_old_generation_then_self_heals_after_unsupported_regression()
 {
    if !isolated_process(
        "no_environment_rollout_waits_for_old_generation_then_self_heals_after_unsupported_regression",
    ) {
        return;
    }
    let local = undeclared_runtime();
    initialize(local.clone());
    let db = crate::test_utils::connect_transaction_test_database("automatic_rollout").await;
    ensure_indexes(&db).await.unwrap();
    publish_observation(&db, &local, CAPABILITIES)
        .await
        .unwrap();
    let cap = models::REGISTRY_READINESS;
    let mut old = row(&local, Utc::now());
    old.id = "old".into();
    old.generation_id = old.id.clone();
    old.capabilities.retain(|c| c != cap);
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .insert_one(&old)
        .await
        .unwrap();
    let start = Instant::now();
    record_activations(&db, &local, &[cap], start)
        .await
        .unwrap();
    assert!(matches!(
        require_serving(&db, cap).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(local.pending_seconds(cap, start).is_none());
    expire(&db, "old").await;
    record_activations(&db, &local, &[cap], start + ACTIVATION_SETTLE_WINDOW)
        .await
        .unwrap();
    assert!(!report(&db, cap).await.unwrap().activated);
    assess_every_ten_seconds(
        &db,
        &local,
        &[cap],
        start + ACTIVATION_SETTLE_WINDOW,
        start + 2 * ACTIVATION_SETTLE_WINDOW,
    )
    .await;
    require_serving(&db, cap).await.unwrap();
    require_flag_enable(&db, cap).await.unwrap();
    let first = report(&db, cap).await.unwrap().active_since;
    let mut sibling = Runtime::new(local.identity.clone(), RolloutConfig::default(), false);
    sibling.identity.generation_id = "supported-new".into();
    publish_observation(&db, &sibling, CAPABILITIES)
        .await
        .unwrap();
    assert!(report_for(&db, &sibling, cap).await.unwrap().serving_ready);
    old.observed_at = Utc::now();
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .replace_one(doc! {"_id":"old"}, &old)
        .await
        .unwrap();
    assert!(matches!(
        require_serving(&db, cap).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        require_flag_enable(&db, cap).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        models::require_execution_ready(&db).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    expire(&db, "old").await;
    require_serving(&db, cap).await.unwrap();
    require_flag_enable(&db, cap).await.unwrap();
    assert_eq!(report(&db, cap).await.unwrap().active_since, first);
}

#[tokio::test]
async fn hard_observation_failures_refuse_serving_flag_enable_and_model_publication() {
    if !isolated_process(
        "hard_observation_failures_refuse_serving_flag_enable_and_model_publication",
    ) {
        return;
    }
    let local = undeclared_runtime();
    initialize(local.clone());
    for case in [
        "unsupported",
        "protocol",
        "executing_missing",
        "future",
        "malformed",
        "malformed_evidence",
        "bound",
        "database",
    ] {
        let db =
            crate::test_utils::connect_transaction_test_database("hard_publication_gate").await;
        ensure_indexes(&db).await.unwrap();
        publish_observation(&db, &local, CAPABILITIES)
            .await
            .unwrap();
        for cap in [
            models::REGISTRY_READINESS,
            models::EXECUTION_READINESS,
            models::BILLING_READINESS,
        ] {
            insert_activation(&db, cap).await.unwrap();
        }
        let (actor, model, tariff) = model_fixture(&db).await;
        let replicas = db.collection::<bson::Document>(COLLECTION_NAME);
        match case {
            "unsupported" => {
                replicas
                    .update_many(doc! {}, doc! {"$set":{"capabilities":[]}})
                    .await
                    .unwrap();
            }
            "protocol" => {
                replicas
                    .update_many(doc! {}, doc! {"$set":{"protocol":2}})
                    .await
                    .unwrap();
            }
            "executing_missing" => {
                replicas.delete_many(doc! {}).await.unwrap();
            }
            "future" => {
                replicas.update_many(doc! {}, doc! {"$set":{"observed_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(30))}}).await.unwrap();
            }
            "malformed" => {
                replicas
                    .insert_one(doc! {"_id":"bad","observed_at":bson::DateTime::now()})
                    .await
                    .unwrap();
            }
            "malformed_evidence" => {
                // Corrupt each prerequisite under test: plain tariff publication
                // reads billing evidence, independently of the registry record.
                db.collection::<bson::Document>(ROLLOUTS)
                    .update_many(
                        doc! {"record_kind":RECORD_KIND},
                        doc! {"$set":{"active_since":"bad"}},
                    )
                    .await
                    .unwrap();
            }
            "bound" => {
                replicas
                    .insert_many((0..256).map(|i| {
                        let mut r = row(&local, Utc::now());
                        r.id = format!("extra-{i}");
                        r.generation_id = r.id.clone();
                        bson::to_document(&r).unwrap()
                    }))
                    .await
                    .unwrap();
            }
            _ => {}
        }
        let db = if case == "database" {
            db.client().database_with_options(
                db.name(),
                mongodb::options::DatabaseOptions::builder()
                    .read_concern(mongodb::options::ReadConcern::custom(
                        "invalid-read-concern",
                    ))
                    .build(),
            )
        } else {
            db
        };
        assert!(
            require_serving(&db, models::REGISTRY_READINESS)
                .await
                .is_err(),
            "{case}"
        );
        assert!(
            require_flag_enable(&db, models::FLAG).await.is_err(),
            "{case}"
        );
        assert!(
            models::update(&db, &actor, &model.id, model_update(&model))
                .await
                .is_err(),
            "{case}"
        );
        assert!(
            model_tariffs::create(&db, &actor, &model.id, tariff)
                .await
                .is_err(),
            "{case}"
        );
    }
}

#[tokio::test]
async fn settled_activation_remains_readable_and_usable_under_2422_rules() {
    // Frozen activation reader from f22e7048e; persisted fields must stay compatible.
    #[derive(serde::Deserialize)]
    struct ProductionActivation {
        #[serde(rename = "_id")]
        id: String,
        record_kind: String,
        capability: String,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        active_since: DateTime<Utc>,
    }
    for declared in [false, true] {
        let db =
            crate::test_utils::connect_transaction_test_database("production_readiness_reader")
                .await;
        ensure_indexes(&db).await.unwrap();
        let mut local = undeclared_runtime();
        if declared {
            local.config = RolloutConfig::parse(&local.identity.instance_name, true).unwrap();
        }
        publish_observation(&db, &local, CAPABILITIES)
            .await
            .unwrap();
        let mut old = row(&local, Utc::now());
        old.id = "production-generation".into();
        old.generation_id = old.id.clone();
        db.collection::<SecurityReplica>(COLLECTION_NAME)
            .insert_one(&old)
            .await
            .unwrap();
        let cap = models::REGISTRY_READINESS;
        let start = Instant::now();
        record_activations(&db, &local, &[cap], start)
            .await
            .unwrap();
        assess_every_ten_seconds(&db, &local, &[cap], start, start + ACTIVATION_SETTLE_WINDOW)
            .await;
        // Exact #2422 rollout_state query, including its cutover-marker branch.
        let documents: Vec<bson::Document> = db
            .collection(ROLLOUTS)
            .find(doc! {"$or": [
                {"_id": PLATFORM_CUTOVER},
                {"record_kind": RECORD_KIND, "capability": {"$in": [cap]}},
            ]})
            .limit(3)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0]
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["_id", "record_kind", "capability", "active_since"])
        );
        let evidence: ProductionActivation = bson::from_document(documents[0].clone()).unwrap();
        uuid::Uuid::parse_str(&evidence.id).unwrap();
        assert_eq!(evidence.record_kind, RECORD_KIND);
        assert_eq!(evidence.capability, cap);
        let new_report = report_for(&db, &local, cap).await.unwrap();
        assert_eq!(new_report.active_since, Some(evidence.active_since));
        assert!(!new_report.ready && new_report.serving_ready);
        let production = Runtime::new(
            ReplicaIdentity {
                generation_id: old.generation_id,
                ..local.identity.clone()
            },
            local.config.clone(),
            false,
        );
        let state = report_for(&db, &production, cap).await.unwrap();
        // #2422: ready OR (activated AND no hard blockers); false drain was hard.
        let old_hard_empty = state.serving_blockers.is_empty()
            && !state
                .blockers
                .contains(&"old_replicas_not_declared_drained".into());
        let old_serving = state.ready || (state.activated && old_hard_empty);
        assert_eq!(old_serving, declared);
        assert!(
            !state.ready,
            "#2422 flag enables/publication retain strict refusal during overlap"
        );
    }
}

#[tokio::test]
async fn uninitialized_runtime_refuses_serving_flag_enable_and_publication() {
    if !isolated_process("uninitialized_runtime_refuses_serving_flag_enable_and_publication") {
        return;
    }
    assert!(runtime().is_err());
    let db =
        crate::test_utils::connect_transaction_test_database("uninitialized_publication").await;
    let (actor, model, tariff) = model_fixture(&db).await;
    assert!(matches!(
        require_serving(&db, models::REGISTRY_READINESS).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        require_flag_enable(&db, models::FLAG).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        models::update(&db, &actor, &model.id, model_update(&model)).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        model_tariffs::create(&db, &actor, &model.id, tariff).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
}
