use super::*;
use crate::models::service_account::{CurationGrant, ServiceAccountPurpose};

async fn fixture(
    label: &str,
    budget: i64,
) -> (Database, ServiceAccount, SkillActor, Vec<DownstreamService>) {
    let db = crate::test_utils::connect_transaction_test_database(label).await;
    ensure_indexes(&db).await.unwrap();
    let mut services = Vec::new();
    for _ in 0..2 {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = format!("curation-{}", service.id);
        service.recommended_skills = None;
        db.collection::<DownstreamService>(SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        services.push(service);
    }
    let (mut sa, _) = super::super::service_account_service::create_service_account(
        &db,
        "Editor",
        None,
        "catalog:skills:read catalog:skills:write",
        &[],
        None,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    let now = Utc::now();
    sa.purpose = ServiceAccountPurpose::Curation;
    sa.platform_protected = true;
    sa.curation_grant = Some(CurationGrant {
        id: Uuid::new_v4().to_string(),
        service_ids: services.iter().map(|s| s.id.clone()).collect(),
        ornn_proxy_service_id: None,
        issued_by: sa.created_by.clone(),
        issued_at: now,
        expires_at: None,
        max_writes: budget,
        window_seconds: 3600,
        window_started_at: now,
        writes_used: 0,
    });
    db.collection::<ServiceAccount>(ACCOUNTS)
        .replace_one(doc! {"_id": &sa.id}, &sa)
        .await
        .unwrap();
    let token = ServiceAccountToken {
        id: Uuid::new_v4().to_string(),
        jti: Uuid::new_v4().to_string(),
        service_account_id: sa.id.clone(),
        scope: sa.allowed_scopes.clone(),
        expires_at: now + chrono::Duration::hours(1),
        revoked: false,
        credential_generation: 0,
        created_at: now,
    };
    db.collection::<ServiceAccountToken>(TOKENS)
        .insert_one(&token)
        .await
        .unwrap();
    let actor = SkillActor::Curation {
        id: sa.id.clone(),
        scope: token.scope,
        token_jti: token.jti,
    };
    (db, sa, actor, services)
}

fn names(names: &[&str]) -> SkillUpdate {
    SkillUpdate {
        recommended_skills: Some(names.iter().map(|s| (*s).into()).collect()),
        ..Default::default()
    }
}

async fn write(
    db: &Database,
    service: &str,
    actor: &SkillActor,
    update: &SkillUpdate,
    base: i64,
    request: &str,
) -> AppResult<SkillCommit> {
    commit(
        db,
        service,
        actor,
        update,
        base,
        request,
        &doc! {},
        &doc! {},
        None,
        None,
    )
    .await
}

async fn stored(db: &Database, id: &str) -> DownstreamService {
    db.collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
}

async fn used(db: &Database, id: &str) -> i64 {
    db.collection::<ServiceAccount>(ACCOUNTS)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
        .curation_grant
        .unwrap()
        .writes_used
}

#[tokio::test]
async fn curation_assign_reassign_unassign_clear_restore_and_replay() {
    let (db, sa, actor, services) = fixture("curation_history", 10).await;
    let id = &services[0].id;
    // Exercise missing legacy revision, not just a stored zero.
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id": id},
            doc! {"$unset": {"skills_revision": "", "recommended_skill_refs": ""}},
        )
        .await
        .unwrap();
    let request = Uuid::new_v4().to_string();
    let assigned = write(
        &db,
        id,
        &actor,
        &names(&["other-publisher/setup", "operations"]),
        0,
        &request,
    )
    .await
    .unwrap();
    assert_eq!(assigned.revision, 1);
    let repeated = write(
        &db,
        id,
        &actor,
        &names(&["other-publisher/setup", "operations"]),
        0,
        &request,
    )
    .await
    .unwrap();
    assert!(repeated.replayed);
    assert_eq!(repeated.state, assigned.state);
    assert!(matches!(
        write(&db, id, &actor, &names(&["different"]), 0, &request).await,
        Err(AppError::Conflict(_))
    ));
    for (base, next) in [
        (1, vec!["replacement", "operations"]),
        (2, vec!["operations"]),
        (3, vec![]),
    ] {
        assert_eq!(
            write(
                &db,
                id,
                &actor,
                &names(&next),
                base,
                &Uuid::new_v4().to_string()
            )
            .await
            .unwrap()
            .revision,
            base + 1
        );
    }
    let restored = commit(
        &db,
        id,
        &actor,
        &SkillUpdate::default(),
        4,
        &Uuid::new_v4().to_string(),
        &doc! {},
        &doc! {},
        Some(1),
        None,
    )
    .await
    .unwrap();
    assert_eq!(restored.state, assigned.state);
    let baseline = commit(
        &db,
        id,
        &actor,
        &SkillUpdate::default(),
        5,
        &Uuid::new_v4().to_string(),
        &doc! {},
        &doc! {},
        Some(0),
        None,
    )
    .await
    .unwrap();
    assert_eq!(baseline.state, SkillState::default());
    assert_eq!(used(&db, &sa.id).await, 6);
    assert_eq!(
        db.collection::<Document>(HISTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        6
    );
    assert_eq!(
        db.collection::<Document>(OPERATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        6
    );
}

#[tokio::test]
async fn curation_exact_refs_and_explicit_names_transition() {
    let (db, _, actor, services) = fixture("curation_refs", 10).await;
    let reference = SkillReference {
        source: "ornn".into(),
        skill_id: Uuid::new_v4().to_string(),
        name: "setup".into(),
        version: "1.5".into(),
        sha256: "a".repeat(64),
        dependencies: vec![],
    };
    let input = SkillUpdate {
        recommended_skill_refs: Some(vec![reference.clone()]),
        ..Default::default()
    };
    let first = write(
        &db,
        &services[0].id,
        &actor,
        &input,
        0,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(first.state.recommended_skills, Some(vec!["setup".into()]));
    assert!(matches!(
        write(
            &db,
            &services[0].id,
            &actor,
            &names(&["new"]),
            1,
            &Uuid::new_v4().to_string()
        )
        .await,
        Err(AppError::ValidationError(_))
    ));
    let mut second = input.clone();
    second.recommended_skill_refs.as_mut().unwrap()[0].version = "1.6".into();
    let second = write(
        &db,
        &services[0].id,
        &actor,
        &second,
        1,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert_ne!(
        manifest_digest(&first.state),
        manifest_digest(&second.state)
    );
    let cleared = write(
        &db,
        &services[0].id,
        &actor,
        &SkillUpdate {
            clear_refs: true,
            ..names(&["advisory"])
        },
        2,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(cleared.state.recommended_skill_refs, None);
    let restored = commit(
        &db,
        &services[0].id,
        &actor,
        &SkillUpdate::default(),
        3,
        &Uuid::new_v4().to_string(),
        &doc! {},
        &doc! {},
        Some(1),
        None,
    )
    .await
    .unwrap();
    assert_eq!(restored.state, first.state);
    let mut mutable = input;
    mutable.recommended_skill_refs.as_mut().unwrap()[0].version = "latest".into();
    assert!(resolve_update(&SkillState::default(), &mutable).is_err());
    assert!(resolve_update(&SkillState::default(), &names(&["same", "same"])).is_err());
}

#[tokio::test]
async fn curation_noop_and_replay_do_not_spend_exhausted_budget() {
    let (db, sa, actor, services) = fixture("curation_noop", 1).await;
    let request = Uuid::new_v4().to_string();
    write(&db, &services[0].id, &actor, &names(&["a"]), 0, &request)
        .await
        .unwrap();
    assert!(
        write(&db, &services[0].id, &actor, &names(&["a"]), 0, &request)
            .await
            .unwrap()
            .replayed
    );
    let noop_id = Uuid::new_v4().to_string();
    let noop = write(&db, &services[0].id, &actor, &names(&["a"]), 1, &noop_id)
        .await
        .unwrap();
    assert!(!noop.changed);
    assert_eq!(noop.revision, 1);
    assert_eq!(used(&db, &sa.id).await, 1);
    assert_eq!(
        db.collection::<Document>(HISTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(OPERATIONS)
            .count_documents(doc! {"request_id": &noop_id})
            .await
            .unwrap(),
        0
    );
    assert!(matches!(
        write(&db, &services[0].id, &actor, &names(&["b"]), 1, &noop_id).await,
        Err(AppError::RateLimited)
    ));
    // Persistent fixed window can be reset on another process's next write.
    db.collection::<Document>(ACCOUNTS).update_one(doc! {"_id": &sa.id}, doc! {"$set": {"curation_grant.window_started_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(2))}}).await.unwrap();
    write(&db, &services[0].id, &actor, &names(&["b"]), 1, &noop_id)
        .await
        .unwrap();
    assert_eq!(used(&db, &sa.id).await, 1);
}

#[tokio::test]
async fn curation_concurrent_writers_and_shared_budget() {
    let (db, sa, actor, services) = fixture("curation_budget_race", 1).await;
    let a = names(&["a"]);
    let b = names(&["b"]);
    let ra = Uuid::new_v4().to_string();
    let rb = Uuid::new_v4().to_string();
    let independent_client =
        mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    let other_db = independent_client.database(db.name());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let hook_a = transactions::TransactionCollisionHook::new(barrier.clone());
    let hook_b = transactions::TransactionCollisionHook::new(barrier);
    let (left, right) = tokio::join!(
        COLLISION.scope(
            hook_a.clone(),
            write(&db, &services[0].id, &actor, &a, 0, &ra)
        ),
        COLLISION.scope(
            hook_b.clone(),
            write(&other_db, &services[1].id, &actor, &b, 0, &rb)
        )
    );
    assert!(hook_a.attempts() + hook_b.attempts() >= 3);
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert!(
        matches!(left, Err(AppError::RateLimited)) || matches!(right, Err(AppError::RateLimited))
    );
    assert_eq!(used(&db, &sa.id).await, 1);
    let human = SkillActor::Human {
        id: Uuid::new_v4().to_string(),
    };
    let base = stored(&db, &services[0].id).await.skills_revision;
    let human_a = names(&["human-a"]);
    let human_b = names(&["human-b"]);
    let (left, right) = tokio::join!(
        write(&db, &services[0].id, &human, &human_a, base, &ra),
        write(&db, &services[0].id, &human, &human_b, base, &rb)
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert!(
        matches!(left, Err(AppError::Conflict(_))) || matches!(right, Err(AppError::Conflict(_)))
    );
}

#[tokio::test]
async fn curation_human_cross_service_request_collision_and_concurrent_create() {
    let (db, _, _, services) = fixture("curation_receipt_race", 10).await;
    let human_id = Uuid::new_v4().to_string();
    let human = SkillActor::Human {
        id: human_id.clone(),
    };
    let request = Uuid::new_v4().to_string();
    let input = names(&["same"]);
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let hook_a = transactions::TransactionCollisionHook::new(barrier.clone());
    let hook_b = transactions::TransactionCollisionHook::new(barrier);
    let (left, right) = tokio::join!(
        COLLISION.scope(
            hook_a.clone(),
            write(&db, &services[0].id, &human, &input, 0, &request)
        ),
        COLLISION.scope(
            hook_b.clone(),
            write(&db, &services[1].id, &human, &input, 0, &request)
        )
    );
    assert!(hook_a.attempts() + hook_b.attempts() >= 3);
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert!(
        matches!(left, Err(AppError::Conflict(_))) || matches!(right, Err(AppError::Conflict(_)))
    );
    assert_eq!(
        db.collection::<Document>(HISTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let mut first = services[0].clone();
    first.id = Uuid::new_v4().to_string();
    first.slug = "create-race".into();
    let mut second = first.clone();
    second.id = Uuid::new_v4().to_string();
    let request = Uuid::new_v4().to_string();
    let fingerprint = create_fingerprint(&serde_json::json!({"name": "create-race"})).unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let hook_a = transactions::TransactionCollisionHook::new(barrier.clone());
    let hook_b = transactions::TransactionCollisionHook::new(barrier);
    let (a, b) = tokio::join!(
        COLLISION.scope(
            hook_a,
            create(&db, &first, &human_id, &input, &request, &fingerprint)
        ),
        COLLISION.scope(
            hook_b,
            create(&db, &second, &human_id, &input, &request, &fingerprint)
        )
    );
    assert_eq!(a.unwrap().id, b.unwrap().id);
    assert_eq!(
        db.collection::<Document>(SERVICES)
            .count_documents(doc! {"slug": "create-race"})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn curation_mixed_metadata_atomicity_identity_filter_and_unchanged_skills_receipt() {
    let (db, sa, _, services) = fixture("curation_mixed", 10).await;
    let human = SkillActor::Human {
        id: Uuid::new_v4().to_string(),
    };
    let id = &services[0].id;
    let input = names(&["a"]);
    let request = Uuid::new_v4().to_string();
    let first = commit(
        &db,
        id,
        &human,
        &input,
        0,
        &request,
        &doc! {"name": "new name"},
        &doc! {},
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(first.service.name, "new name");
    let request = Uuid::new_v4().to_string();
    let mixed = commit(
        &db,
        id,
        &human,
        &input,
        1,
        &request,
        &doc! {"name": "second name"},
        &doc! {},
        None,
        None,
    )
    .await
    .unwrap();
    assert!(!mixed.changed);
    assert_eq!(mixed.revision, 1);
    assert_eq!(used(&db, &sa.id).await, 0);
    assert_eq!(
        db.collection::<Document>(HISTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(
        commit(
            &db,
            id,
            &human,
            &input,
            1,
            &request,
            &doc! {"name": "second name"},
            &doc! {},
            None,
            None
        )
        .await
        .unwrap()
        .replayed
    );
    assert!(matches!(
        commit(
            &db,
            id,
            &human,
            &input,
            1,
            &request,
            &doc! {"name": "different"},
            &doc! {},
            None,
            None
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let invalid_filter =
        doc! {"updated_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::days(1))};
    assert!(matches!(
        commit(
            &db,
            id,
            &human,
            &names(&["b"]),
            1,
            &Uuid::new_v4().to_string(),
            &doc! {"name": "must not commit"},
            &invalid_filter,
            None,
            None
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(stored(&db, id).await.name, "second name");
    assert_eq!(stored(&db, id).await.skills_revision, 1);
}

#[tokio::test]
async fn curation_failed_history_insert_rolls_back_service_budget_token_and_receipt() {
    let (db, sa, actor, services) = fixture("curation_abort", 10).await;
    db.run_command(doc! {"collMod": HISTORY, "validator": {"revision": -1}, "validationLevel": "strict", "validationAction": "error"}).await.unwrap();
    assert!(
        write(
            &db,
            &services[0].id,
            &actor,
            &names(&["rollback"]),
            0,
            &Uuid::new_v4().to_string()
        )
        .await
        .is_err()
    );
    assert_eq!(stored(&db, &services[0].id).await.skills_revision, 0);
    assert_eq!(stored(&db, &services[0].id).await.recommended_skills, None);
    assert_eq!(used(&db, &sa.id).await, 0);
    assert_eq!(
        db.collection::<Document>(OPERATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let token = db
        .collection::<Document>(TOKENS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(!token.contains_key("curation_write_fence"));
}

#[tokio::test]
async fn curation_live_authority_is_checked_before_replay() {
    let (db, sa, actor, services) = fixture("curation_live", 10).await;
    let request = Uuid::new_v4().to_string();
    let input = names(&["a"]);
    write(&db, &services[0].id, &actor, &input, 0, &request)
        .await
        .unwrap();
    assert!(matches!(
        write(
            &db,
            &Uuid::new_v4().to_string(),
            &actor,
            &input,
            0,
            &request
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    for update in [
        doc! {"curation_grant.expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))},
        doc! {"is_active": false},
        doc! {"credential_generation": 1_i64},
    ] {
        db.collection::<Document>(ACCOUNTS)
            .update_one(doc! {"_id": &sa.id}, doc! {"$set": update})
            .await
            .unwrap();
        assert!(
            write(&db, &services[0].id, &actor, &input, 0, &request)
                .await
                .is_err()
        );
        db.collection::<ServiceAccount>(ACCOUNTS)
            .replace_one(doc! {"_id": &sa.id}, &sa)
            .await
            .unwrap();
    }
    db.collection::<Document>(TOKENS)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(matches!(
        write(&db, &services[0].id, &actor, &input, 0, &request).await,
        Err(AppError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn curation_revoke_after_transaction_reads_fences_token_and_grant() {
    for revoke_token in [true, false] {
        let (db, sa, actor, services) = fixture("curation_revoke_race", 10).await;
        let reached = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let resume = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let input = names(&["must-not-commit"]);
        let request = Uuid::new_v4().to_string();
        let writer = AUTHORITY_PAUSE.scope(
            (reached.clone(), resume.clone(), once),
            write(&db, &services[0].id, &actor, &input, 0, &request),
        );
        let revoker = async {
            reached.wait().await;
            if revoke_token {
                db.collection::<Document>(TOKENS)
                    .update_many(
                        doc! {"service_account_id":&sa.id},
                        doc! {"$set":{"revoked":true}},
                    )
                    .await
                    .unwrap();
            } else {
                super::super::curation_grant_service::revoke(&db, &sa.id)
                    .await
                    .unwrap();
            }
            resume.wait().await;
        };
        let (result, ()) = tokio::join!(writer, revoker);
        assert!(
            matches!(
                result,
                Err(AppError::Unauthorized(_)) | Err(AppError::Forbidden(_))
            ),
            "unexpected result: {:?}",
            result.err()
        );
        assert_eq!(stored(&db, &services[0].id).await.skills_revision, 0);
        assert_eq!(
            db.collection::<Document>(HISTORY)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(OPERATIONS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        if revoke_token {
            assert_eq!(used(&db, &sa.id).await, 0);
        }
    }
}

#[tokio::test]
async fn catalog_editor_concurrent_role_revocation_fences_write() {
    let (db, mut sa, actor, services) = fixture("editor_role_revoke_race", 10).await;
    let role_id = Uuid::new_v4().to_string();
    let roles = db.collection::<Document>(crate::models::role::COLLECTION_NAME);
    roles.insert_one(doc! {"_id": &role_id, "client_id": null, "permissions": [super::super::catalog_editor_service::WRITE_PERMISSION]}).await.unwrap();
    sa.purpose = ServiceAccountPurpose::CatalogEditor;
    sa.role_ids = vec![role_id.clone()];
    db.collection::<ServiceAccount>(ACCOUNTS)
        .replace_one(doc! {"_id": &sa.id}, &sa)
        .await
        .unwrap();
    let reached = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let input = names(&["must-not-commit"]);
    let request = Uuid::new_v4().to_string();
    let writer = AUTHORITY_PAUSE.scope(
        (reached.clone(), resume.clone(), once),
        write(&db, &services[0].id, &actor, &input, 0, &request),
    );
    let revoker = async {
        reached.wait().await;
        roles
            .update_one(doc! {"_id": &role_id}, doc! {"$set": {"permissions": []}})
            .await
            .unwrap();
        resume.wait().await;
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(writer, revoker)
    })
    .await
    .expect("role revocation race must finish");
    assert!(
        matches!(result, Err(AppError::Forbidden(_))),
        "unexpected result: {:?}",
        result.err()
    );
    assert_eq!(stored(&db, &services[0].id).await.skills_revision, 0);
    for collection in [HISTORY, OPERATIONS] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn catalog_editor_noop_and_replay_do_not_mutate_role_fence() {
    let (db, mut sa, actor, services) = fixture("editor_noop_role_fence", 10).await;
    let role_id = Uuid::new_v4().to_string();
    let roles = db.collection::<Document>(crate::models::role::COLLECTION_NAME);
    roles.insert_one(doc! {"_id": &role_id, "client_id": null, "permissions": [super::super::catalog_editor_service::WRITE_PERMISSION]}).await.unwrap();
    sa.purpose = ServiceAccountPurpose::CatalogEditor;
    sa.role_ids = vec![role_id.clone()];
    sa.rate_limit_override = Some(1);
    db.collection::<ServiceAccount>(ACCOUNTS)
        .replace_one(doc! {"_id": &sa.id}, &sa)
        .await
        .unwrap();
    let input = names(&["stable"]);
    let request = Uuid::new_v4().to_string();
    write(&db, &services[0].id, &actor, &input, 0, &request)
        .await
        .unwrap();
    let before = roles
        .find_one(doc! {"_id": &role_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.get_i64("catalog_editor_write_fence").unwrap(), 1);
    let replay = write(&db, &services[0].id, &actor, &input, 0, &request)
        .await
        .unwrap();
    assert!(replay.replayed);
    let noop = write(
        &db,
        &services[0].id,
        &actor,
        &input,
        1,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert!(!noop.mutated);
    assert_eq!(
        roles
            .find_one(doc! {"_id": &role_id})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    // Freeze the test window in the future so machine speed cannot reset it.
    db.collection::<Document>(ACCOUNTS).update_one(doc! {"_id": &sa.id}, doc! {"$set": {"catalog_editor_write_window": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::minutes(1))}}).await.unwrap();
    let next = names(&["changed"]);
    let next_request = Uuid::new_v4().to_string();
    assert!(matches!(
        write(&db, &services[0].id, &actor, &next, 1, &next_request).await,
        Err(AppError::RateLimited)
    ));
    assert_eq!(stored(&db, &services[0].id).await.skills_revision, 1);
    assert_eq!(
        roles
            .find_one(doc! {"_id": &role_id})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    db.collection::<Document>(ACCOUNTS).update_one(doc! {"_id": &sa.id}, doc! {"$set": {"catalog_editor_write_window": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(2))}}).await.unwrap();
    assert!(
        write(&db, &services[0].id, &actor, &next, 1, &next_request)
            .await
            .unwrap()
            .changed
    );
    let account = db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id": &sa.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.get_i64("catalog_editor_writes_used").unwrap(), 1);
}

#[tokio::test]
async fn catalog_editor_legacy_generation_writes_and_rotation_rejects_old_token() {
    let (db, mut sa, actor, services) = fixture("editor_legacy_generation", 10).await;
    let role_id = Uuid::new_v4().to_string();
    db.collection::<Document>(crate::models::role::COLLECTION_NAME)
        .insert_one(doc! {"_id": &role_id, "client_id": null, "permissions": [super::super::catalog_editor_service::WRITE_PERMISSION]})
        .await.unwrap();
    sa.purpose = ServiceAccountPurpose::CatalogEditor;
    sa.role_ids = vec![role_id];
    let mut legacy = bson::to_document(&sa).unwrap();
    legacy.remove("credential_generation");
    db.collection::<Document>(ACCOUNTS)
        .replace_one(doc! {"_id": &sa.id}, legacy)
        .await
        .unwrap();
    let result = write(
        &db,
        &services[0].id,
        &actor,
        &names(&["legacy-write"]),
        0,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert!(result.changed);
    assert_eq!(result.revision, 1);
    let (rotated, _) = super::super::service_account_service::rotate_secret(&db, &sa.id, true)
        .await
        .unwrap();
    assert_eq!(rotated.credential_generation, 1);
    let result = write(
        &db,
        &services[0].id,
        &actor,
        &names(&["stale-token-write"]),
        1,
        &Uuid::new_v4().to_string(),
    )
    .await;
    assert!(matches!(result, Err(AppError::Unauthorized(_))));
    assert_eq!(stored(&db, &services[0].id).await.skills_revision, 1);
    for collection in [HISTORY, OPERATIONS] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }
}
