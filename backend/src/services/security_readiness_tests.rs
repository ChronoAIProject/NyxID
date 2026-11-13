use super::*;
use crate::services::feature_flag_service as flags;

fn local(name: &str, generation: &str, roster: &[&str], required: bool) -> Runtime {
    Runtime {
        identity: ReplicaIdentity {
            instance_name: name.into(),
            generation_id: generation.into(),
            internal_base_url: "http://127.0.0.1:3002".into(),
        },
        config: RolloutConfig::parse(&roster.join(","), true).unwrap(),
        platform_policy_required: required,
    }
}
fn row(local: &Runtime, now: DateTime<Utc>) -> SecurityReplica {
    SecurityReplica {
        id: local.identity.generation_id.clone(),
        instance_name: local.identity.instance_name.clone(),
        generation_id: local.identity.generation_id.clone(),
        protocol: 1,
        capabilities: CAPABILITIES.iter().map(|c| (*c).into()).collect(),
        expected_replicas: local.config.expected_replicas.clone(),
        drained: local.config.drained,
        platform_policy_required: local.platform_policy_required,
        observed_at: now,
    }
}

#[test]
fn roster_parser_rejects_ambiguous_or_unbounded_inputs() {
    for value in ["a,a", "a,", "a b", "a,,b"] {
        assert!(RolloutConfig::parse(value, true).is_err(), "{value}");
    }
    assert_eq!(
        RolloutConfig::parse(" b, a ", true)
            .unwrap()
            .expected_replicas,
        ["a", "b"]
    );
    assert!(
        RolloutConfig::parse("", false)
            .unwrap()
            .expected_replicas
            .is_empty()
    );
    assert!(RolloutConfig::parse(&"a".repeat(129), true).is_err());
    let too_many = (0..129)
        .map(|i| format!("replica-{i}"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(RolloutConfig::parse(&too_many, true).is_err());
}

#[test]
fn readiness_binds_executing_generation_roster_and_observation() {
    let now = Utc::now();
    let a = local("a", "a1", &["a", "b"], false);
    let b = local("b", "b1", &["a", "b"], false);
    let good = vec![row(&a, now), row(&b, now)];
    assert!(assess(&good, &a, CAPABILITIES[0], false, now).ready);
    for change in 0..8 {
        let mut rows = good.clone();
        match change {
            0 => {
                rows.remove(0);
            }
            1 => rows[0].observed_at = now - Duration::seconds(46),
            2 => rows.push(row(&local("a", "a2", &["a", "b"], false), now)),
            3 => rows[1].capabilities.clear(),
            4 => rows[1].expected_replicas = vec!["b".into()],
            5 => rows[1].observed_at = now + Duration::seconds(6),
            6 => rows[1].drained = false,
            _ => rows[1].protocol = 2,
        }
        assert!(
            !assess(&rows, &a, CAPABILITIES[0], false, now).ready,
            "case {change}"
        );
    }
    let unrelated = local("other", "other1", &["other"], false);
    assert!(!assess(&[row(&unrelated, now)], &a, CAPABILITIES[0], false, now).ready);
    let restarted = local("a", "a2", &["a", "b"], false);
    assert!(!assess(&good, &restarted, CAPABILITIES[0], false, now).ready);
}

#[tokio::test]
async fn effective_flag_reads_preserve_existing_override_and_clear_when_unready() {
    let db = crate::test_utils::connect_test_database("readiness_effective")
        .await
        .expect("configured Mongo required");
    let user = uuid::Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &user,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let flag = flags::ASSISTANT_LIVE_TURN_GATE_FLAG_KEY;
    ready_test_replica(&db).await;
    flags::set_platform_override(&db, flag, &flags::FlagTarget::Global, true, &user)
        .await
        .unwrap();
    flags::set_platform_override(
        &db,
        flag,
        &flags::FlagTarget::User(user.clone()),
        false,
        &user,
    )
    .await
    .unwrap();
    assert!(
        !flags::personal_flag_enabled(&db, &user, flag)
            .await
            .unwrap()
    );
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    flags::clear_platform_override(&db, flag, &flags::FlagTarget::User(user.clone()))
        .await
        .unwrap();
    assert!(
        flags::personal_flag_enabled(&db, &user, flag)
            .await
            .unwrap()
    );
    ready_test_replica(&db).await;
    assert!(
        flags::personal_flag_enabled(&db, &user, flag)
            .await
            .unwrap()
    );
    // A foreign roster is still unready, but cannot break an active flag read.
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    let foreign = local("other", "other1", &["other"], false);
    observe(&db, &foreign.identity, &foreign.config, false)
        .await
        .unwrap();
    assert!(
        flags::personal_flag_enabled(&db, &user, flag)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn membership_enablement_preserves_configured_flags_when_readiness_becomes_unready() {
    use crate::models::{
        org_membership::{MemberScopeSource, OrgMembership, OrgRole},
        user::UserType,
    };
    let db = crate::test_utils::connect_test_database("readiness_membership")
        .await
        .expect("configured Mongo required");
    let user = uuid::Uuid::new_v4().to_string();
    let org = uuid::Uuid::new_v4().to_string();
    for (id, kind) in [(&user, UserType::Person), (&org, UserType::Org)] {
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(id, kind))
            .await
            .unwrap();
    }
    let flag = "assistant:org-agents";
    ready_test_replica(&db).await;
    flags::set_platform_org_override(&db, &org, flag, true, &user)
        .await
        .unwrap();
    assert!(
        !flags::personal_flag_enabled(&db, &user, flag)
            .await
            .unwrap()
    );
    let membership = OrgMembership {
        id: uuid::Uuid::new_v4().to_string(),
        org_user_id: org,
        member_user_id: user.clone(),
        role: OrgRole::Member,
        scope_source: MemberScopeSource::Inherit,
        allowed_service_ids: None,
        created_at: Utc::now(),
        revoked_at: None,
    };
    db.collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME)
        .insert_one(&membership)
        .await
        .unwrap();
    // Losing readiness after activation does not break newly matching members.
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(
        flags::resolve_personal_features(&db, &user)
            .await
            .unwrap()
            .iter()
            .any(|key| key == flag)
    );

    super::super::org_agent_service::require_creation_enabled(&db, &user)
        .await
        .unwrap();
    db.collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME)
        .delete_one(doc! {"_id": &membership.id})
        .await
        .unwrap();
    assert!(
        !flags::personal_flag_enabled(&db, &user, flag)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn observation_query_indexes_age_and_rejects_excess_live_generations() {
    let db = crate::test_utils::connect_test_database("readiness_query_bounds")
        .await
        .expect("configured Mongo required");
    ensure_indexes(&db).await.unwrap();
    let collection = db.collection::<SecurityReplica>(COLLECTION_NAME);
    let indexes: Vec<_> = collection
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        indexes
            .iter()
            .any(|index| index.keys == doc! {"observed_at": 1})
    );
    let fixture = local("test", "test1", &["test"], false);
    let old = Utc::now() - Duration::seconds(46);
    let historical: Vec<_> = (0..300)
        .map(|i| row(&local("old", &format!("old{i}"), &["old"], false), old))
        .collect();
    collection.insert_many(historical).await.unwrap();
    observe(&db, &fixture.identity, &fixture.config, false)
        .await
        .unwrap();
    assert_eq!(rows(&db).await.unwrap().len(), 1);
    assert!(
        report_for(&db, &fixture, CAPABILITIES[0])
            .await
            .unwrap()
            .ready
    );
    let live: Vec<_> = (0..256)
        .map(|i| {
            row(
                &local("extra", &format!("extra{i}"), &["extra"], false),
                Utc::now(),
            )
        })
        .collect();
    collection.insert_many(live).await.unwrap();
    assert!(matches!(
        rows(&db).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(report_for(&db, &fixture, CAPABILITIES[0]).await.is_err());
}

#[tokio::test]
async fn platform_cutover_survives_enabled_replica_expiry_and_local_downgrade() {
    let db = crate::test_utils::connect_test_database("readiness_cutover")
        .await
        .expect("configured Mongo required");
    let enabled = local("a", "a1", &["a", "b"], true);
    let disabled = local("b", "b1", &["a", "b"], false);
    observe(&db, &enabled.identity, &enabled.config, true)
        .await
        .unwrap();
    observe(&db, &disabled.identity, &disabled.config, false)
        .await
        .unwrap();
    assert!(
        !report_for(&db, &disabled, PLATFORM_POLICY)
            .await
            .unwrap()
            .ready
    );
    assert!(
        !report_for(&db, &disabled, PLATFORM_POLICY)
            .await
            .unwrap()
            .serving_ready
    );
    db.collection::<SecurityReplica>(COLLECTION_NAME).update_one(doc! {"_id": "a1"}, doc! {"$set": {"observed_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(46))}}).await.unwrap();
    assert!(platform_cutover_required(&db).await.unwrap());
    assert!(matches!(
        require_platform(&db, false).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(
        !report_for(&db, &disabled, PLATFORM_POLICY)
            .await
            .unwrap()
            .ready
    );
}

#[tokio::test]
async fn actual_process_platform_policy_and_catalog_inventory() {
    for value in ["true", "false"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services::security_readiness_service::tests::platform_policy_process_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("NYXID_TEST_PLATFORM_POLICY_CHILD", "1")
            .env("PLATFORM_REQUIRE_OPERATION_POLICY", value)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child {value}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}

#[tokio::test]
#[ignore = "entrypoint run by actual_process_platform_policy_and_catalog_inventory with isolated process environment"]
async fn platform_policy_process_child() {
    assert_eq!(
        std::env::var("NYXID_TEST_PLATFORM_POLICY_CHILD").as_deref(),
        Ok("1")
    );
    let required = super::super::proxy_service::actor_addressed_policy_required();
    let local = local("child", "child1", &["child"], required);
    initialize(local.clone());
    let db = crate::test_utils::connect_test_database("platform_policy_process")
        .await
        .expect("configured Mongo required");
    observe(&db, &local.identity, &local.config, required)
        .await
        .unwrap();
    let mut service = crate::test_utils::test_auto_connected_catalog_service();
    service.auth_method = "bearer".into();
    service.service_category = "internal".into();
    service.credential_encrypted = vec![1, 2, 3];
    let collection = db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    );
    collection.insert_one(&service).await.unwrap();
    let actor = super::super::proxy_service::EffectiveActor::from_user_id("actor");
    let first =
        super::super::proxy_service::authorize_master_credential(&db, &service, &actor).await;
    assert_eq!(
        first.is_err(),
        required,
        "actual process env must affect authorization"
    );
    let inventory = report_for(&db, &local, PLATFORM_POLICY).await.unwrap();
    assert_eq!(inventory.catalog_missing_policy, 1);
    assert!(!inventory.ready);
    assert!(!inventory.serving_ready);
    service.proxy_operation_policy =
        Some(crate::models::downstream_service::ProxyOperationPolicy { rules: vec![] });
    collection
        .replace_one(doc! {"_id": &service.id}, &service)
        .await
        .unwrap();
    let inventory = report_for(&db, &local, PLATFORM_POLICY).await.unwrap();
    assert_eq!(inventory.catalog_missing_policy, 0);
    assert_eq!(inventory.catalog_explicit_empty_policy, 1);
    assert_eq!(inventory.ready, required);
    assert!(
        super::super::proxy_service::authorize_master_credential(&db, &service, &actor)
            .await
            .is_ok()
    );
    if required {
        observe(&db, &local.identity, &local.config, true)
            .await
            .unwrap();
        let active = report_for(&db, &local, PLATFORM_POLICY).await.unwrap();
        assert!(active.activated && active.serving_ready);
        let sibling = local.clone();
        let mut overlap = row(&sibling, Utc::now());
        overlap.id = "overlap".into();
        overlap.generation_id = "overlap".into();
        db.collection::<SecurityReplica>(COLLECTION_NAME)
            .insert_one(&overlap)
            .await
            .unwrap();
        assert!(require_platform(&db, true).await.is_ok());
        let mut false_local = local.clone();
        false_local.platform_policy_required = false;
        assert!(
            !report_for(&db, &false_local, PLATFORM_POLICY)
                .await
                .unwrap()
                .serving_ready
        );
        collection
            .update_one(
                doc! {"_id": &service.id},
                doc! {"$unset": {"proxy_operation_policy": ""}},
            )
            .await
            .unwrap();
        let refused = report_for(&db, &local, PLATFORM_POLICY).await.unwrap();
        assert!(refused.activated && !refused.serving_ready);
        assert!(
            refused
                .serving_blockers
                .contains(&"catalog_master_policy_missing".to_owned())
        );
    }
    // An explicit empty policy passes credential resolution but denies an
    // actual operation through the existing operation evaluator.
    let policy = service.proxy_operation_policy.as_ref().unwrap();
    assert!(policy.rules.is_empty());
    let path =
        crate::services::proxy_authorization::CanonicalPath::from_mcp_literal("/items").unwrap();
    assert!(
        crate::services::proxy_authorization::authorize_proxy_operation(&service, "GET", &path)
            .is_err()
    );
    db.drop().await.unwrap();
}

#[test]
fn protected_org_and_hosted_intent_writes_require_every_replica_capability_and_drain() {
    let now = Utc::now();
    let a = local("a", "a1", &["a", "b"], false);
    let b = local("b", "b1", &["a", "b"], false);
    for capability in [
        flags::ORG_INSTRUCTION_FLAG_KEY,
        flags::HOSTED_CONNECT_INTENT_FLAG_KEY,
    ] {
        let good = vec![row(&a, now), row(&b, now)];
        assert!(assess(&good, &a, capability, false, now).ready);
        let mut old_writer = good.clone();
        old_writer[1].capabilities.retain(|c| c != capability);
        assert!(!assess(&old_writer, &a, capability, false, now).ready);
        let mut undrained = good;
        undrained[1].drained = false;
        assert!(!assess(&undrained, &a, capability, false, now).ready);
    }
}

#[test]
fn automation_capability_requires_every_replica_to_opt_in() {
    let now = Utc::now();
    let a = local("a", "a1", &["a", "b"], false);
    let b = local("b", "b1", &["a", "b"], false);
    let mut rows = vec![row(&a, now), row(&b, now)];
    assert!(!CAPABILITIES.contains(&AUTOMATION_COMPLETION));
    assert!(!assess(&rows, &a, AUTOMATION_COMPLETION, false, now).ready);
    rows[0].capabilities.push(AUTOMATION_COMPLETION.into());
    assert!(!assess(&rows, &a, AUTOMATION_COMPLETION, false, now).ready);
    rows[1].capabilities.push(AUTOMATION_COMPLETION.into());
    assert!(assess(&rows, &a, AUTOMATION_COMPLETION, false, now).ready);
    rows[1].drained = false;
    assert!(!assess(&rows, &a, AUTOMATION_COMPLETION, false, now).ready);
}

fn roster_cases(now: DateTime<Utc>) -> Vec<(Runtime, Vec<SecurityReplica>, &'static str)> {
    let a = local("a", "a1", &["a"], false);
    let no_roster = local("a", "a1", &[], false);
    vec![
        (
            a.clone(),
            vec![row(&a, now), row(&local("a", "a2", &["a"], false), now)],
            "missing_or_duplicate_replica:a",
        ),
        (
            local("a", "a1", &["a", "b"], false),
            vec![row(&local("a", "a1", &["a", "b"], false), now)],
            "missing_or_duplicate_replica:b",
        ),
        (
            a.clone(),
            vec![row(&a, now), row(&local("b", "b1", &["a"], false), now)],
            "undeclared_replica:b",
        ),
        (
            a.clone(),
            vec![row(&local("a", "a1", &["a", "b"], false), now)],
            "inconsistent_serving_roster",
        ),
        (
            no_roster.clone(),
            vec![row(&no_roster, now)],
            "serving_roster_not_declared",
        ),
    ]
}

#[test]
fn each_roster_warning_requires_prior_activation_and_never_allows_activation() {
    let now = Utc::now();
    for (local, rows, warning) in roster_cases(now) {
        for active in [None, Some(now)] {
            let report = assess(&rows, &local, CAPABILITIES[0], false, now).with_activation(active);
            assert!(!report.ready, "{warning}");
            assert_eq!(report.activated, active.is_some());
            assert_eq!(report.serving_ready, active.is_some(), "{warning}");
            assert_eq!(report.serving_with_roster_warnings, active.is_some());
            assert!(report.roster_warnings.iter().any(|s| s == warning));
        }
    }
}

#[test]
fn every_live_generation_must_support_capability_even_with_activation() {
    let now = Utc::now();
    for (name, roster) in [("a", vec!["a"]), ("b", vec!["a"]), ("b", vec!["a", "b"])] {
        let a = local("a", "a1", &roster, false);
        let b = local(name, "b1", &roster, false);
        for protocol in [false, true] {
            let mut unsupported = row(&b, now);
            if protocol {
                unsupported.protocol = 2;
            } else {
                unsupported.capabilities.clear();
            }
            let report = assess(
                &[row(&a, now), unsupported],
                &a,
                CAPABILITIES[0],
                false,
                now,
            )
            .with_activation(Some(now));
            assert!(!report.serving_ready);
            assert!(
                report
                    .serving_blockers
                    .contains(&format!("unsupported_replica:{name}"))
            );
        }
    }
}

#[test]
fn every_hard_assessment_blocker_still_refuses_serving() {
    let now = Utc::now();
    let a = local("a", "a1", &["a"], true);
    for case in 0..8 {
        let mut local = a.clone();
        let mut rows = vec![row(&a, now)];
        let (capability, cutover, code) = match case {
            0 => {
                rows.clear();
                (CAPABILITIES[0], false, "executing_process_not_observed")
            }
            1 => {
                local.config.drained = false;
                (CAPABILITIES[0], false, "old_replicas_not_declared_drained")
            }
            2 => {
                rows[0].drained = false;
                (CAPABILITIES[0], false, "old_replicas_not_declared_drained")
            }
            3 => {
                rows[0].observed_at = now + Duration::seconds(6);
                (CAPABILITIES[0], false, "future_observation:a")
            }
            4 => {
                rows[0].platform_policy_required = false;
                (
                    PLATFORM_POLICY,
                    true,
                    "platform_policy_configuration_mismatch",
                )
            }
            5 => {
                local.platform_policy_required = false;
                (
                    PLATFORM_POLICY,
                    true,
                    "executing_process_platform_policy_disabled",
                )
            }
            6 => {
                local.platform_policy_required = false;
                rows[0].platform_policy_required = false;
                (PLATFORM_POLICY, false, "platform_policy_not_enabled")
            }
            _ => {
                rows[0].observed_at = now - Duration::seconds(46);
                (CAPABILITIES[0], false, "executing_process_not_observed")
            }
        };
        let report = assess(&rows, &local, capability, cutover, now).with_activation(Some(now));
        assert!(!report.serving_ready, "{code}");
        assert!(report.serving_blockers.iter().any(|s| s == code), "{code}");
    }
}

#[tokio::test]
async fn activation_is_strict_durable_and_concurrent_heartbeats_keep_one_timestamp() {
    let db = crate::test_utils::connect_test_database("readiness_activation")
        .await
        .unwrap();
    ensure_indexes(&db).await.unwrap();
    let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
    let capability = CAPABILITIES[0];
    for (local, rows, warning) in roster_cases(Utc::now()) {
        replicas.delete_many(doc! {}).await.unwrap();
        replicas.insert_many(rows).await.unwrap();
        record_clean_activations(&db, &local, &[capability])
            .await
            .unwrap();
        let report = report_for(&db, &local, capability).await.unwrap();
        assert!(!report.activated && !report.serving_ready, "{warning}");
    }
    let a = local("a", "a1", &["a"], false);
    replicas.delete_many(doc! {}).await.unwrap();
    replicas.insert_one(row(&a, Utc::now())).await.unwrap();
    // Reads may serve a clean fleet but never write activation evidence.
    let before = report_for(&db, &a, capability).await.unwrap();
    assert!(before.ready && before.serving_ready && !before.activated);
    let barrier = tokio::sync::Barrier::new(16);
    let results = futures::future::join_all((0..16).map(|_| async {
        barrier.wait().await;
        insert_activation(&db, capability).await
    }))
    .await;
    for result in results {
        result.unwrap();
    }
    let records: Vec<_> = db
        .collection::<SecurityCapabilityActivation>(ROLLOUTS)
        .find(doc! {"record_kind": RECORD_KIND})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        uuid::Uuid::parse_str(&records[0].id)
            .unwrap()
            .get_version_num(),
        4
    );
    let since = records[0].active_since;
    for generation in ["old-overlap", "new-overlap"] {
        replicas
            .insert_one(row(&local("a", generation, &["a"], false), Utc::now()))
            .await
            .unwrap();
        record_clean_activations(&db, &a, &[capability, AUTOMATION_COMPLETION])
            .await
            .unwrap();
        let report = report_for(&db, &a, capability).await.unwrap();
        assert!(report.activated && report.serving_ready && report.serving_with_roster_warnings);
        assert!(!report.ready);
        assert_eq!(report.active_since, Some(since));
        assert!(
            !report_for(&db, &a, AUTOMATION_COMPLETION)
                .await
                .unwrap()
                .activated
        );
    }
    // A replacement executing process inherits fleet-shared activation, both
    // during overlap and after the old generations expire.
    let replacement = local("a", "new-overlap", &["a"], false);
    assert!(
        report_for(&db, &replacement, capability)
            .await
            .unwrap()
            .serving_ready
    );
    replicas
        .update_many(
            doc! {"_id": {"$ne": "new-overlap"}},
            doc! {"$set": {
                "observed_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(46))
            }},
        )
        .await
        .unwrap();
    let drained = report_for(&db, &replacement, capability).await.unwrap();
    assert!(drained.ready && drained.serving_ready);
    assert_eq!(drained.active_since, Some(since));
    // Reintroduce overlap before removing activation evidence.
    replicas
        .insert_one(row(&local("a", "returning", &["a"], false), Utc::now()))
        .await
        .unwrap();
    // Every request re-reads MongoDB: deleting evidence cannot leave cached authority.
    db.collection::<SecurityCapabilityActivation>(ROLLOUTS)
        .delete_many(doc! {"record_kind": RECORD_KIND})
        .await
        .unwrap();
    assert!(
        !report_for(&db, &replacement, capability)
            .await
            .unwrap()
            .serving_ready
    );
}

#[tokio::test]
async fn clean_serving_keeps_two_read_commands_and_no_writes() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let commands = Arc::new(Mutex::new(Vec::new()));
    let captured = commands.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            captured.lock().unwrap().push(event.command_name);
        }
    });
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "readiness_commands",
        handler,
    )
    .await
    .unwrap();
    let heartbeat = controlled_ready_test_replica(&db).await;
    heartbeat.abort();
    commands.lock().unwrap().clear();
    baseline_require(&db, CAPABILITIES[0]).await.unwrap();
    let baseline = commands.lock().unwrap().clone();
    assert_eq!(baseline, ["find", "find"]);
    commands.lock().unwrap().clear();
    require_serving(&db, CAPABILITIES[0]).await.unwrap();
    assert_eq!(*commands.lock().unwrap(), baseline);
    commands.lock().unwrap().clear();
    require_activation(&db, CAPABILITIES[0]).await.unwrap();
    assert_eq!(*commands.lock().unwrap(), baseline);
    eprintln!(
        "main require = {baseline:?}; require_serving = {baseline:?}; require_activation = {baseline:?}"
    );
}

// Execute the pre-fix command sequence from 31f3d8969e051e1fe6ed2f42a5594ded90606fe5.
// The pure assessment and catalog queries are unchanged. The distinguishing
// legacy operation is the exact-ID find_one, which ignores new activation rows.
async fn baseline_require(db: &Database, capability: &str) -> AppResult<()> {
    let mut report = assess(
        &rows(db).await?,
        runtime()?,
        capability,
        db.collection::<bson::Document>(ROLLOUTS)
            .find_one(doc! {"_id":"platform-policy-v1"})
            .await?
            .is_some(),
        Utc::now(),
    );
    inventory(db, capability, &mut report).await?;
    if report.ready {
        Ok(())
    } else {
        Err(AppError::SecurityReadinessUnavailable)
    }
}

async fn baseline_require_platform(db: &Database, local_required: bool) -> AppResult<()> {
    let rows = rows(db).await?;
    if local_required
        || platform_cutover_required(db).await?
        || rows.iter().any(|r| r.platform_policy_required)
    {
        baseline_require(db, PLATFORM_POLICY).await?;
    }
    Ok(())
}

#[tokio::test]
async fn platform_command_counts_match_main_and_old_query_ignores_activation_rows() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "services::security_readiness_service::tests::runtime_process_child",
            "--ignored",
            "--nocapture",
        ])
        .env("NYXID_TEST_READINESS_CHILD", "commands")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
}

#[tokio::test]
async fn serving_diagnostics_are_shared_throttled_chained_and_failure_is_nonfatal() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tracing_subscriber::prelude::*;
    struct Warnings(Arc<AtomicUsize>);
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warnings {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if *event.metadata().level() == tracing::Level::WARN
                && event.metadata().fields().field("warnings").is_some()
            {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    let warnings = Arc::new(AtomicUsize::new(0));
    let subscriber =
        tracing::Dispatch::new(tracing_subscriber::registry().with(Warnings(warnings.clone())));
    use crate::models::{audit_log::AuditLog, coordination::RATE_WINDOW_COLLECTION_NAME};
    let db = crate::test_utils::connect_test_database("readiness_diagnostics")
        .await
        .unwrap();
    let heartbeat = controlled_ready_test_replica(&db).await;
    heartbeat.abort();
    let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
    let mut overlap = replicas.find_one(doc! {}).await.unwrap().unwrap();
    overlap.id = uuid::Uuid::new_v4().to_string();
    overlap.generation_id = overlap.id.clone();
    replicas.insert_one(overlap).await.unwrap();
    let capability = CAPABILITIES[0];
    let report = report(&db, capability).await.unwrap();
    crate::test_utils::ensure_rate_window_headroom(
        diagnostics::INTERVAL,
        std::time::Duration::from_secs(5),
    )
    .await;
    tracing::dispatcher::with_default(&subscriber, || diagnostics::warn(&db, capability, &report))
        .unwrap()
        .await
        .unwrap();
    assert!(
        tracing::dispatcher::with_default(&subscriber, || diagnostics::warn(
            &db, capability, &report
        ))
        .is_none()
    );
    assert!(diagnostics::warn(&db, CAPABILITIES[1], &report).is_none());
    diagnostics::reset_local(&db, &report);
    tracing::dispatcher::with_default(&subscriber, || {
        diagnostics::warn(&db, CAPABILITIES[1], &report)
    })
    .unwrap()
    .await
    .unwrap();
    let audits = db.collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME);
    let rows: Vec<_> = audits
        .find(doc! {"event_type": diagnostics::EVENT})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(warnings.load(Ordering::SeqCst), 1);
    assert!(rows[0].seq.is_some() && rows[0].entry_hash.is_some());
    assert_eq!(
        rows[0].event_data.as_ref().unwrap()["first_observed_capability"],
        capability
    );
    // Simulate the next shared window without an hour-long test.
    let windows = db.collection::<bson::Document>(RATE_WINDOW_COLLECTION_NAME);
    windows
        .delete_many(doc! {"namespace": diagnostics::EVENT})
        .await
        .unwrap();
    diagnostics::reset_local(&db, &report);
    tracing::dispatcher::with_default(&subscriber, || diagnostics::warn(&db, capability, &report))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(
        audits
            .count_documents(doc! {"event_type": diagnostics::EVENT})
            .await
            .unwrap(),
        2
    );
    windows
        .update_many(
            doc! {"namespace": diagnostics::EVENT},
            doc! {"$set": {"count":"malformed"}},
        )
        .await
        .unwrap();
    diagnostics::reset_local(&db, &report);
    tracing::dispatcher::with_default(&subscriber, || diagnostics::warn(&db, capability, &report))
        .unwrap()
        .await
        .unwrap();
    require_serving(&db, capability).await.unwrap();
    assert_eq!(
        audits
            .count_documents(doc! {"event_type": diagnostics::EVENT})
            .await
            .unwrap(),
        2
    );
    // A failing audit append is also isolated from execution, including the
    // hash-chain writer's transaction. Existing audit rows remain untouched.
    windows
        .delete_many(doc! {"namespace": diagnostics::EVENT})
        .await
        .unwrap();
    db.run_command(doc! {"collMod": crate::models::audit_log::COLLECTION_NAME,
    "validator": {"event_type": {"$ne": diagnostics::EVENT}}})
        .await
        .unwrap();
    diagnostics::reset_local(&db, &report);
    tracing::dispatcher::with_default(&subscriber, || diagnostics::warn(&db, capability, &report))
        .unwrap()
        .await
        .unwrap();
    require_serving(&db, capability).await.unwrap();
    assert_eq!(
        audits
            .count_documents(doc! {"event_type": diagnostics::EVENT})
            .await
            .unwrap(),
        2
    );
    db.run_command(doc! {"collMod": crate::models::audit_log::COLLECTION_NAME,
    "validator": {}})
        .await
        .unwrap();
    let mut changed = report;
    changed
        .roster_warnings
        .push("undeclared_replica:new".into());
    diagnostics::warn(&db, capability, &changed)
        .unwrap()
        .await
        .unwrap();
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":diagnostics::EVENT})
            .await
            .unwrap(),
        3
    );
    changed.roster_warnings.reverse();
    assert!(diagnostics::warn(&db, CAPABILITIES[1], &changed).is_none());
}

#[tokio::test]
async fn activation_refuses_hard_failures_and_serving_never_ignores_database_errors() {
    let db = crate::test_utils::connect_test_database("readiness_errors")
        .await
        .unwrap();
    ensure_indexes(&db).await.unwrap();
    let a = local("a", "a1", &["a"], false);
    let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
    for case in 0..4 {
        let mut row = row(&a, Utc::now());
        match case {
            0 => row.drained = false,
            1 => row.capabilities.clear(),
            2 => row.observed_at = Utc::now() + Duration::seconds(10),
            _ => row.observed_at = Utc::now() - Duration::seconds(46),
        }
        replicas
            .replace_one(doc! {"_id": &row.id}, row)
            .upsert(true)
            .await
            .unwrap();
        record_clean_activations(&db, &a, &[CAPABILITIES[0]])
            .await
            .unwrap();
        assert!(
            !report_for(&db, &a, CAPABILITIES[0])
                .await
                .unwrap()
                .activated
        );
    }
    let heartbeat = controlled_ready_test_replica(&db).await;
    heartbeat.abort();
    let raw = db.collection::<bson::Document>(COLLECTION_NAME);
    raw.insert_one(doc! {"_id":"malformed", "observed_at":bson::DateTime::now()})
        .await
        .unwrap();
    assert!(require_serving(&db, CAPABILITIES[0]).await.is_err());
    raw.delete_one(doc! {"_id":"malformed"}).await.unwrap();
    let mut options =
        mongodb::options::ClientOptions::parse("mongodb://127.0.0.1:1/?directConnection=true")
            .await
            .unwrap();
    options.server_selection_timeout = Some(std::time::Duration::from_millis(100));
    let unavailable = mongodb::Client::with_options(options)
        .unwrap()
        .database("unavailable");
    assert!(
        require_serving(&unavailable, CAPABILITIES[0])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn activated_automation_enrollment_serves_overlap_and_keeps_hard_loss_fallback() {
    use crate::services::trigger_schedule::continuation;
    let db = crate::test_utils::connect_test_database("automation_roster_overlap")
        .await
        .unwrap();
    ready_automation_test_replica(&db).await;
    let sibling = overlapping_test_replica(&db).await;
    assert!(continuation::enrollment_ready(&db, true).await);
    assert!(!continuation::enrollment_ready(&db, false).await);
    let report = report(&db, AUTOMATION_COMPLETION).await.unwrap();
    assert!(report.activated && report.serving_ready && !report.ready);
    remove_test_replica_capability(&db, &sibling, AUTOMATION_COMPLETION).await;
    assert!(!continuation::enrollment_ready(&db, true).await);
}

#[tokio::test]
async fn recording_failure_does_not_fail_observation_or_skip_other_capabilities() {
    let db = crate::test_utils::connect_test_database("activation_retry")
        .await
        .unwrap();
    ensure_indexes(&db).await.unwrap();
    let local = local("a", "a1", &["a"], false);
    let blocked = CAPABILITIES[0];
    let other = CAPABILITIES[1];
    db.run_command(doc! {"collMod":ROLLOUTS, "validator":{"capability":{"$ne":blocked}}})
        .await
        .unwrap();
    observe(&db, &local.identity, &local.config, false)
        .await
        .unwrap();
    assert_eq!(rows(&db).await.unwrap().len(), 1);
    assert!(!report_for(&db, &local, blocked).await.unwrap().activated);
    assert!(report_for(&db, &local, other).await.unwrap().activated);
    db.run_command(doc! {"collMod":ROLLOUTS, "validator":{}})
        .await
        .unwrap();
    observe(&db, &local.identity, &local.config, false)
        .await
        .unwrap();
    assert!(report_for(&db, &local, blocked).await.unwrap().activated);
    let activations = db.collection::<bson::Document>(ROLLOUTS);
    activations
        .update_one(
            doc! {"capability":blocked},
            doc! {"$set":{"active_since":"malformed"}},
        )
        .await
        .unwrap();
    activations
        .delete_one(doc! {"capability":other})
        .await
        .unwrap();
    observe(&db, &local.identity, &local.config, false)
        .await
        .unwrap();
    assert!(report_for(&db, &local, blocked).await.is_err());
    assert!(report_for(&db, &local, other).await.unwrap().activated);
    let excessive: Vec<_> = (0..256)
        .map(|i| {
            row(
                &self::local("extra", &format!("e{i}"), &["a"], false),
                Utc::now(),
            )
        })
        .collect();
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .insert_many(excessive)
        .await
        .unwrap();
    observe(&db, &local.identity, &local.config, false)
        .await
        .unwrap();
    assert!(rows(&db).await.is_err());
}

#[tokio::test]
async fn unknown_flag_activation_is_ungated_but_unknown_execution_fails_closed() {
    let db = crate::test_utils::connect_test_database("unknown_capability")
        .await
        .unwrap();
    require_activation(&db, "ordinary-custom-flag")
        .await
        .unwrap();
    assert!(matches!(
        require_serving(&db, "ordinary-custom-flag").await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
}

#[tokio::test]
async fn duplicate_activation_without_unique_index_fails_closed() {
    let db = crate::test_utils::connect_test_database("duplicate_activation")
        .await
        .unwrap();
    let local = local("a", "a1", &["a"], false);
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .insert_one(row(&local, Utc::now()))
        .await
        .unwrap();
    let records: Vec<_> = (0..2)
        .map(|_| SecurityCapabilityActivation {
            id: uuid::Uuid::new_v4().to_string(),
            record_kind: RECORD_KIND.into(),
            capability: CAPABILITIES[0].into(),
            active_since: Utc::now(),
        })
        .collect();
    db.collection::<SecurityCapabilityActivation>(ROLLOUTS)
        .insert_many(records)
        .await
        .unwrap();
    assert!(matches!(
        report_for(&db, &local, CAPABILITIES[0]).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
}

#[test]
fn additive_diagnostics_are_sorted_without_changing_legacy_blocker_order() {
    let now = Utc::now();
    let local = local("a", "a1", &["a", "missing"], false);
    let undeclared = self::local("z", "z1", &["z"], false);
    let mut report = assess(
        &[row(&local, now), row(&undeclared, now)],
        &local,
        PLATFORM_POLICY,
        false,
        now,
    );
    report.block(Blocker::CatalogMasterPolicyMissing);
    let legacy = report.blockers.clone();
    let report = report.with_activation(None);
    assert_eq!(report.blockers, legacy);
    assert_eq!(
        report.blockers.last().map(String::as_str),
        Some("catalog_master_policy_missing")
    );
    assert!(
        report
            .serving_blockers
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
    );
    assert!(
        report
            .roster_warnings
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
    );
    assert_eq!(report.roster_warnings.len(), 3);
}

#[tokio::test]
async fn isolated_runtime_and_each_admin_roster_case() {
    for case in ["uninitialized", "0", "1", "2", "3", "4"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services::security_readiness_service::tests::runtime_process_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("NYXID_TEST_READINESS_CHILD", case)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child {case}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}

#[tokio::test]
#[ignore = "isolated runtime entrypoint run by isolated_runtime_and_each_admin_roster_case"]
async fn runtime_process_child() {
    let case = std::env::var("NYXID_TEST_READINESS_CHILD").unwrap();
    if case == "commands" {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{Arc, Mutex};
        let commands = Arc::new(Mutex::new(Vec::new()));
        let captured = commands.clone();
        let handler = EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                captured
                    .lock()
                    .unwrap()
                    .push((event.command_name, event.command));
            }
        });
        let db = crate::test_utils::connect_test_database_with_command_handler(
            "platform_command_counts",
            handler,
        )
        .await
        .unwrap();
        let local = local("a", "a1", &["a"], true);
        initialize(local.clone());
        ensure_indexes(&db).await.unwrap();
        observe(&db, &local.identity, &local.config, true)
            .await
            .unwrap();
        assert!(
            db.collection::<bson::Document>(ROLLOUTS)
                .count_documents(doc! {"record_kind":RECORD_KIND})
                .await
                .unwrap()
                > 0
        );
        for required in [true, false] {
            commands.lock().unwrap().clear();
            baseline_require_platform(&db, required).await.unwrap();
            let baseline = commands.lock().unwrap().clone();
            let before: Vec<_> = baseline.iter().map(|(name, _)| name.clone()).collect();
            assert_eq!(before.len(), if required { 5 } else { 6 });
            assert!(baseline.iter().any(|(name, cmd)| name == "find"
                && cmd.get_str("find").ok() == Some(ROLLOUTS)
                && cmd.get_document("filter").ok() == Some(&doc! {"_id":"platform-policy-v1"})));
            commands.lock().unwrap().clear();
            require_platform(&db, required).await.unwrap();
            let after: Vec<_> = commands
                .lock()
                .unwrap()
                .iter()
                .map(|(name, _)| name.clone())
                .collect();
            assert_eq!(before, after);
            eprintln!(
                "require_platform(local_required={required}): main={before:?}, fixed={after:?}"
            );
        }
        let mut overlap = row(&local, Utc::now());
        overlap.id = "overlap".into();
        overlap.generation_id = overlap.id.clone();
        db.collection::<SecurityReplica>(COLLECTION_NAME)
            .insert_one(overlap)
            .await
            .unwrap();
        assert!(matches!(
            baseline_require(&db, PLATFORM_POLICY).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        require_platform(&db, true).await.unwrap();
        eprintln!(
            "old exact-ID cutover query ignores activation rows; old gate remains strict during overlap"
        );
        return;
    }
    let db = crate::test_utils::connect_test_database("readiness_runtime_child")
        .await
        .unwrap();
    let flag = flags::ASSISTANT_LIVE_TURN_GATE_FLAG_KEY;
    if case == "uninitialized" {
        assert!(matches!(
            require_serving(&db, flag).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        assert!(matches!(
            require_activation(&db, flag).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        return;
    }
    ensure_indexes(&db).await.unwrap();
    // Record a genuine previous clean deployment before changing its roster.
    let clean = local("a", "a1", &["a"], false);
    observe(&db, &clean.identity, &clean.config, false)
        .await
        .unwrap();
    let (local, rows, warning) = roster_cases(Utc::now()).remove(case.parse::<usize>().unwrap());
    initialize(local);
    let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
    replicas.delete_many(doc! {}).await.unwrap();
    replicas.insert_many(rows).await.unwrap();
    let report = report(&db, flag).await.unwrap();
    assert!(
        report.activated && report.serving_ready && !report.ready,
        "{warning}"
    );
    let user = uuid::Uuid::new_v4().to_string();
    let org = uuid::Uuid::new_v4().to_string();
    for (id, kind) in [
        (&user, crate::models::user::UserType::Person),
        (&org, crate::models::user::UserType::Org),
    ] {
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(id, kind))
            .await
            .unwrap();
    }
    use crate::models::feature_flag_override::{FeatureFlagOverride, FlagTargetKind};
    use crate::services::{
        audit_service::AuditActor,
        feature_flag_admin_service::{Mutation, mutate},
    };
    let actor = AuditActor {
        user_id: user.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    for (kind, target_id) in [
        (FlagTargetKind::Global, None),
        (FlagTargetKind::User, Some(user.clone())),
        (FlagTargetKind::Org, Some(org)),
    ] {
        assert!(matches!(
            mutate(
                &db,
                flag,
                Mutation::Override {
                    kind,
                    target_id: target_id.clone(),
                    enabled: Some(true)
                },
                &actor
            )
            .await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        let disabled = mutate(
            &db,
            flag,
            Mutation::Override {
                kind,
                target_id,
                enabled: Some(false),
            },
            &actor,
        )
        .await
        .unwrap();
        assert!(!disabled.override_row.unwrap().enabled);
    }
    // Seed an existing global cohort and a personal opt-out, then exercise the
    // production transactional clear-that-enables gate under every warning.
    let enrollment_flag = crate::services::execution_permission_enrollment::FLAG;
    let now = Utc::now();
    let overrides =
        db.collection::<FeatureFlagOverride>(crate::models::feature_flag_override::COLLECTION_NAME);
    for (kind, target_key, enabled) in [
        (FlagTargetKind::Global, None, true),
        (FlagTargetKind::User, Some(user.clone()), false),
    ] {
        overrides
            .insert_one(FeatureFlagOverride {
                id: uuid::Uuid::new_v4().to_string(),
                org_user_id: None,
                flag_key: enrollment_flag.into(),
                definition_id: None,
                target_kind: kind,
                target_key,
                enabled,
                created_at: now,
                updated_at: now,
                updated_by: user.clone(),
            })
            .await
            .unwrap();
    }
    assert!(matches!(
        mutate(
            &db,
            enrollment_flag,
            Mutation::Override {
                kind: FlagTargetKind::User,
                target_id: Some(user.clone()),
                enabled: None,
            },
            &actor
        )
        .await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(
        overrides
            .find_one(doc! {"flag_key":enrollment_flag,"target_key":&user,"enabled":false})
            .await
            .unwrap()
            .is_some(),
        "refused clear must roll back the delete"
    );
}

#[tokio::test]
async fn test_fixture_setup_batches_evidence_and_measures_main_and_round1_cost() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let mut timings = [std::time::Duration::ZERO; 3];
    let mut counts = [0; 3];
    for sample in 0..10 {
        for variant in 0..3 {
            let commands = Arc::new(Mutex::new(Vec::new()));
            let captured = commands.clone();
            let db = crate::test_utils::connect_test_database_with_command_handler(
                &format!("readiness_fixture_cost_{sample}_{variant}"),
                EventHandler::callback(move |event| {
                    if let CommandEvent::Started(event) = event {
                        captured.lock().unwrap().push(event.command_name);
                    }
                }),
            )
            .await
            .unwrap();
            let local = local("test-backend", "test-process", &["test-backend"], false);
            commands.lock().unwrap().clear();
            let start = std::time::Instant::now();
            match variant {
                // Main's helper only published the observation.
                0 => {
                    let replica = row(&local, Utc::now());
                    db.collection::<SecurityReplica>(COLLECTION_NAME)
                        .replace_one(doc! {"_id": &replica.id}, replica)
                        .upsert(true)
                        .await
                        .unwrap();
                }
                // The accepted round-1 helper also built indexes and recorded
                // each capability through the production heartbeat recorder.
                1 => {
                    ensure_indexes(&db).await.unwrap();
                    observe_with_capabilities(
                        &db,
                        &local.identity,
                        &local.config,
                        false,
                        CAPABILITIES,
                    )
                    .await
                    .unwrap();
                }
                _ => {
                    drop(controlled_ready_test_replica(&db).await);
                    stop_test_replica(&db).await;
                }
            }
            timings[variant] += start.elapsed();
            let observed = commands.lock().unwrap().clone();
            counts[variant] = observed.len();
            if variant == 2 {
                assert_eq!(
                    observed
                        .iter()
                        .filter(|name| name.as_str() == "bulkWrite")
                        .count(),
                    1
                );
                assert_eq!(
                    observed
                        .iter()
                        .filter(|name| name.as_str() == "update")
                        .count(),
                    1
                );
                assert!(report(&db, CAPABILITIES[0]).await.unwrap().activated);
            }
        }
    }
    for (variant, name) in ["main", "round1", "batched"].into_iter().enumerate() {
        eprintln!(
            "fixture {name}: commands={}, mean_ms={:.3}",
            counts[variant],
            timings[variant].as_secs_f64() * 100.0
        );
    }
    assert_eq!(counts[0], 1);
    assert!(counts[2] < counts[1]);
}

#[tokio::test]
async fn stopped_fixture_heartbeat_cannot_restore_an_expired_observation() {
    let db = crate::test_utils::connect_test_database("stopped_readiness_fixture")
        .await
        .unwrap();
    ready_test_replica(&db).await;
    stop_test_replica(&db).await;
    let old = bson::DateTime::from_chrono(Utc::now() - Duration::seconds(60));
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set": {"observed_at": old}})
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(11)).await;
    let state = report(&db, CAPABILITIES[0]).await.unwrap();
    assert!(!state.serving_ready);
    assert!(
        state
            .serving_blockers
            .contains(&"executing_process_not_observed".into())
    );
    ready_test_replica(&db).await;
    assert!(report(&db, CAPABILITIES[0]).await.unwrap().serving_ready);
    stop_test_replica(&db).await;
}
