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
