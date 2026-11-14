use super::*;
use crate::{
    models::{feature_flag_override::FlagTargetKind, user::UserType},
    services::{
        audit_service::AuditActor,
        feature_flag_admin_service::{Mutation, mutate},
        feature_flag_service as flags, platform_model_service as models,
    },
};

async fn fixture() -> (Database, AuditActor, String, SecurityReplica) {
    let db = crate::test_utils::connect_transaction_test_database("recorded_flag_enable").await;
    ready_test_replica(&db).await;
    stop_test_replica(&db).await;
    let sibling = overlapping_test_replica(&db).await;
    let actor = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(
        &uuid::Uuid::new_v4().to_string(),
    ));
    let org = uuid::Uuid::new_v4().to_string();
    for (id, kind) in [(&actor.user_id, UserType::Person), (&org, UserType::Org)] {
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(id, kind))
            .await
            .unwrap();
    }
    (db, actor, org, sibling)
}

async fn set(
    db: &Database,
    actor: &AuditActor,
    key: &str,
    kind: FlagTargetKind,
    target_id: Option<String>,
    enabled: Option<bool>,
) -> AppResult<crate::services::feature_flag_admin_service::WriteResult> {
    mutate(
        db,
        key,
        Mutation::Override {
            kind,
            target_id,
            enabled,
        },
        actor,
    )
    .await
}

#[tokio::test]
async fn recorded_flags_enable_globally_for_users_and_orgs_under_overlap_but_refuse_missing_support_or_evidence()
 {
    let families = [
        (
            flags::ASSISTANT_LIVE_TURN_GATE_FLAG_KEY,
            vec![flags::ASSISTANT_LIVE_TURN_GATE_FLAG_KEY],
        ),
        (
            models::FLAG,
            vec![
                models::REGISTRY_READINESS,
                models::EXECUTION_READINESS,
                models::BILLING_READINESS,
            ],
        ),
        (
            flags::EXECUTION_PERMISSIONS_FLAG_KEY,
            vec![
                flags::EXECUTION_PERMISSIONS_FLAG_KEY,
                crate::services::execution_permission_enrollment::READINESS,
                "permissions:execution-dispatch-v1",
                "permissions:execution-authoring-v1",
                "permissions:execution-authoring-v2",
            ],
        ),
    ];
    for (flag, capabilities) in families {
        for capability in capabilities {
            let (db, actor, org, sibling) = fixture().await;
            // A false drain declaration is advisory on the same paths.
            db.collection::<SecurityReplica>(COLLECTION_NAME)
                .update_many(doc! {}, doc! {"$set":{"drained":false}})
                .await
                .unwrap();
            let mut restored = sibling.clone();
            restored.drained = false;
            let mut targets = vec![
                (FlagTargetKind::Global, None),
                (FlagTargetKind::Org, Some(org.clone())),
            ];
            if capability != crate::services::execution_permission_enrollment::READINESS {
                targets.push((FlagTargetKind::User, Some(actor.user_id.clone())));
            }
            for (kind, id) in &targets {
                assert!(
                    set(&db, &actor, flag, *kind, id.clone(), Some(true))
                        .await
                        .unwrap()
                        .override_row
                        .unwrap()
                        .enabled
                );
            }
            for failure in ["unsupported", "unrecorded"] {
                if failure == "unsupported" {
                    remove_test_replica_capability(&db, &sibling, capability).await;
                } else {
                    db.collection::<SecurityReplica>(COLLECTION_NAME)
                        .replace_one(doc! {"_id":&sibling.id}, &restored)
                        .await
                        .unwrap();
                    db.collection::<bson::Document>(ROLLOUTS)
                        .delete_one(doc! {"record_kind":RECORD_KIND,"capability":capability})
                        .await
                        .unwrap();
                }
                for (kind, id) in &targets {
                    assert!(
                        matches!(
                            set(&db, &actor, flag, *kind, id.clone(), Some(true)).await,
                            Err(AppError::SecurityReadinessUnavailable)
                        ),
                        "{flag}/{capability}/{failure}"
                    );
                    assert!(
                        !set(&db, &actor, flag, *kind, id.clone(), Some(false))
                            .await
                            .unwrap()
                            .override_row
                            .unwrap()
                            .enabled
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn recorded_clear_that_enables_preserves_cohort_prerequisites_and_disable_under_overlap() {
    use crate::services::execution_permission_enrollment as enrollment;
    for capability in [
        enrollment::FLAG,
        enrollment::READINESS,
        "permissions:execution-dispatch-v1",
        "permissions:execution-authoring-v1",
    ] {
        let (db, actor, org, sibling) = fixture().await;
        set(
            &db,
            &actor,
            enrollment::FLAG,
            FlagTargetKind::Global,
            None,
            Some(true),
        )
        .await
        .unwrap();
        for (kind, id) in [
            (FlagTargetKind::User, actor.user_id.clone()),
            (FlagTargetKind::Org, org),
        ] {
            set(
                &db,
                &actor,
                enrollment::FLAG,
                kind,
                Some(id.clone()),
                Some(false),
            )
            .await
            .unwrap();
            set(&db, &actor, enrollment::FLAG, kind, Some(id.clone()), None)
                .await
                .unwrap();
            for failure in ["unsupported", "unrecorded"] {
                let disabled = set(
                    &db,
                    &actor,
                    enrollment::FLAG,
                    kind,
                    Some(id.clone()),
                    Some(false),
                )
                .await
                .unwrap()
                .override_row
                .unwrap();
                if failure == "unsupported" {
                    remove_test_replica_capability(&db, &sibling, capability).await;
                } else {
                    db.collection::<SecurityReplica>(COLLECTION_NAME)
                        .replace_one(doc! {"_id":&sibling.id}, &sibling)
                        .await
                        .unwrap();
                    db.collection::<bson::Document>(ROLLOUTS)
                        .delete_one(doc! {"record_kind":RECORD_KIND,"capability":capability})
                        .await
                        .unwrap();
                }
                assert!(
                    matches!(
                        set(&db, &actor, enrollment::FLAG, kind, Some(id.clone()), None).await,
                        Err(AppError::SecurityReadinessUnavailable)
                    ),
                    "{capability}/{failure}"
                );
                let saved = db
                    .collection::<crate::models::feature_flag_override::FeatureFlagOverride>(
                        crate::models::feature_flag_override::COLLECTION_NAME,
                    )
                    .find_one(doc! {"_id": &disabled.id})
                    .await
                    .unwrap();
                assert_eq!(
                    saved,
                    Some(disabled),
                    "refused clear must roll back its delete"
                );
                db.collection::<SecurityReplica>(COLLECTION_NAME)
                    .replace_one(doc! {"_id":&sibling.id}, &sibling)
                    .await
                    .unwrap();
                insert_activation(&db, capability).await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn legacy_override_writers_use_recorded_serving_support_for_all_targets() {
    let (db, actor, org, sibling) = fixture().await;
    let flag = flags::ASSISTANT_LIVE_TURN_GATE_FLAG_KEY;
    for target in [
        flags::FlagTarget::Global,
        flags::FlagTarget::User(actor.user_id.clone()),
    ] {
        assert!(
            flags::set_platform_override(&db, flag, &target, true, &actor.user_id)
                .await
                .unwrap()
                .enabled
        );
        remove_test_replica_capability(&db, &sibling, flag).await;
        assert!(matches!(
            flags::set_platform_override(&db, flag, &target, true, &actor.user_id).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        assert!(
            !flags::set_platform_override(&db, flag, &target, false, &actor.user_id)
                .await
                .unwrap()
                .enabled
        );
        db.collection::<SecurityReplica>(COLLECTION_NAME)
            .replace_one(doc! {"_id":&sibling.id}, &sibling)
            .await
            .unwrap();
    }
    assert!(
        flags::set_platform_org_override(&db, &org, flag, true, &actor.user_id)
            .await
            .unwrap()
            .enabled
    );
    db.collection::<bson::Document>(ROLLOUTS)
        .delete_one(doc! {"record_kind":RECORD_KIND,"capability":flag})
        .await
        .unwrap();
    assert!(matches!(
        flags::set_platform_org_override(&db, &org, flag, true, &actor.user_id).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(
        !flags::set_platform_org_override(&db, &org, flag, false, &actor.user_id)
            .await
            .unwrap()
            .enabled
    );
}

#[tokio::test]
async fn unknown_flag_enables_without_readiness_commands_and_unknown_execution_stays_closed() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(Mutex::new(Vec::new()));
    let captured = commands.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "ungated_flag_commands",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                captured.lock().unwrap().push(event.command);
            }
        }),
    )
    .await
    .unwrap();
    let key = "ordinary:custom-flag";
    let actor = uuid::Uuid::new_v4().to_string();
    flags::create_definition(&db, key, "Ordinary custom flag", false, &actor)
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    require_flag_enable(&db, key).await.unwrap();
    assert!(matches!(
        require_serving(&db, key).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(commands.lock().unwrap().is_empty());
    assert!(
        flags::set_platform_override(&db, key, &flags::FlagTarget::Global, true, &actor)
            .await
            .unwrap()
            .enabled
    );
    assert!(
        !commands
            .lock()
            .unwrap()
            .iter()
            .any(|command| command.values().any(|value| value
                .as_str()
                .is_some_and(|name| name == COLLECTION_NAME || name == ROLLOUTS)))
    );
}

#[tokio::test]
async fn clean_flag_enable_waits_for_evidence_and_admin_command_counts_match_previous_gate() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(Mutex::new(Vec::new()));
    let captured = commands.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "gated_flag_commands",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                captured.lock().unwrap().push(event.command_name);
            }
        }),
    )
    .await
    .unwrap();
    ready_test_replica(&db).await;
    stop_test_replica(&db).await;
    let flag = flags::ASSISTANT_LIVE_TURN_GATE_FLAG_KEY;
    let actor = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(
        &uuid::Uuid::new_v4().to_string(),
    ));
    // Reproduce the old strict gate followed by the unchanged generic override
    // mutation body. false bypasses only the new gate; both first writes append
    // their audit row. This is a command-sequence baseline, not an old binary.
    commands.lock().unwrap().clear();
    assert!(report(&db, flag).await.unwrap().ready);
    set(&db, &actor, flag, FlagTargetKind::Global, None, Some(false))
        .await
        .unwrap();
    let before = commands.lock().unwrap().clone();
    commands.lock().unwrap().clear();
    set(&db, &actor, flag, FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    let after = commands.lock().unwrap().clone();
    assert_eq!(before, after);
    eprintln!("gated admin enable: previous={before:?}; observed-support={after:?}");
    commands.lock().unwrap().clear();
    assert!(report(&db, flag).await.unwrap().ready);
    let before_request = commands.lock().unwrap().clone();
    commands.lock().unwrap().clear();
    require_flag_enable(&db, flag).await.unwrap();
    let after_request = commands.lock().unwrap().clone();
    assert_eq!(before_request, after_request);
    assert_eq!(after_request, ["find", "find"]);
    eprintln!(
        "gated enable readiness commands: previous={before_request:?}; observed-support={after_request:?}"
    );
    db.collection::<bson::Document>(ROLLOUTS)
        .delete_one(doc! {"record_kind":RECORD_KIND,"capability":flag})
        .await
        .unwrap();
    let state = report(&db, flag).await.unwrap();
    assert!(state.ready && !state.activated && !state.serving_ready);
    assert!(matches!(
        require_flag_enable(&db, flag).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        require_serving(&db, flag).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
}
