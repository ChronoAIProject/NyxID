use super::*;
use mongodb::{Database, bson::Document};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

async fn database(name: &str) -> Database {
    crate::test_utils::connect_transaction_test_database(name).await
}
async fn saved(db: &Database, id: &str) -> InviteCode {
    db.collection::<InviteCode>(COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}
async fn expire(db: &Database, id: &str) {
    // Shorten only this fixture's durable deadline, using MongoDB's clock.
    // The baseline has no reservations to age; its unowned increment persists.
    db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":id}, vec![doc! {"$set": {
        "reservations":{"$map":{"input":{"$ifNull":["$reservations",[]]}, "as":"r", "in":{"$mergeObjects":["$$r",{"expires_at":{"$add":["$$NOW",20]}}]}}}
    }}]).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

#[tokio::test]
async fn abandoned_signup_returns_capacity_on_the_next_reservation() {
    let db = database("invite_abandoned").await;
    let code = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let reserved = Notify::new();
    {
        let mut signup = Box::pin(async {
            let _reservation = reserve_invite_code(&db, &code.code, "abandoned@example.test")
                .await
                .unwrap();
            reserved.notify_one();
            std::future::pending::<()>().await;
        });
        tokio::select! {
            _ = &mut signup => panic!("signup must remain before account creation"),
            _ = reserved.notified() => {}
        }
        // Drop the signup: no account insert and no compensation can run.
    }
    assert_eq!(saved(&db, &code.id).await.used_count, 1);
    assert_eq!(
        db.collection::<Document>(USERS_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    expire(&db, &code.id).await;
    let next = reserve_invite_code(&db, &code.code, "next@example.test").await;
    assert!(
        next.is_ok(),
        "abandoned reservation must return capacity: {next:?}"
    );
    assert_eq!(saved(&db, &code.id).await.used_count, 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn signup_storage_command_counts() {
    let commands = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "invite_counts",
        mongodb::event::EventHandler::callback(move |event| {
            if let mongodb::event::command::CommandEvent::Started(e) = event {
                observed.lock().unwrap().push(e.command_name);
            }
        }),
    )
    .await;
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let code = create_invite_code(&db, "admin", 2, None).await.unwrap();
    commands.lock().unwrap().clear();
    crate::services::auth_service::register_user(
        &db,
        "direct@example.test",
        &crate::crypto::token::generate_random_token(),
        None,
        None,
        true,
    )
    .await
    .unwrap();
    let direct = std::mem::take(&mut *commands.lock().unwrap());
    println!("INVITE_COMMANDS password_direct: {direct:?}");
    // Both signup paths read live targets and the two schedule timings.
    assert_eq!(direct.len(), 6);
    let reservation = reserve_invite_code(&db, &code.code, "invited@example.test")
        .await
        .unwrap();
    let user = crate::services::auth_service::register_user(
        &db,
        "invited@example.test",
        &crate::crypto::token::generate_random_token(),
        None,
        Some(&reservation),
        true,
    )
    .await
    .unwrap();
    assert!(user.actually_created);
    let invited = std::mem::take(&mut *commands.lock().unwrap());
    println!("INVITE_COMMANDS password_invited: {invited:?}");
    assert_eq!(invited.len(), 9);
    db.drop().await.unwrap();
}

fn person(email: &str) -> crate::models::user::User {
    let mut user = crate::test_utils::test_user(
        &Uuid::new_v4().to_string(),
        crate::models::user::UserType::Person,
    );
    user.email = email.into();
    user
}
async fn assert_accounting(db: &Database, code: &InviteCode, committed: usize, pending: usize) {
    let stored = saved(db, &code.id).await;
    assert_eq!(stored.used_count as usize, committed + pending);
    assert_eq!(stored.usages.len(), committed);
    assert_eq!(stored.reservations.len(), pending);
    assert_eq!(
        db.collection::<Document>(USERS_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap() as usize,
        committed
    );
}

#[tokio::test]
async fn expired_commit_before_reclaim_keeps_exactly_one_permanent_use() {
    let db = database("invite_commit_first").await;
    let code = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let reservation = reserve_invite_code(&db, &code.code, "slow@example.test")
        .await
        .unwrap();
    expire(&db, &code.id).await;
    commit_signup(&db, &reservation, &person("slow@example.test"))
        .await
        .unwrap();
    assert!(matches!(
        reserve_invite_code(&db, &code.code, "next@example.test").await,
        Err(AppError::InviteCodeExhausted)
    ));
    release_reservation(&db, &reservation).await;
    release_reservation(&db, &reservation).await;
    assert_accounting(&db, &code, 1, 0).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn reclaim_before_commit_refuses_slow_signup_and_cannot_release_successor() {
    let db = database("invite_reclaim_first").await;
    let code = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let old = reserve_invite_code(&db, &code.code, "slow@example.test")
        .await
        .unwrap();
    expire(&db, &code.id).await;
    let next = reserve_invite_code(&db, &code.code, "next@example.test")
        .await
        .unwrap();
    let error = crate::services::auth_service::register_user(
        &db,
        "slow@example.test",
        &crate::crypto::token::generate_random_token(),
        None,
        Some(&old),
        true,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, AppError::InviteCodeInvalid));
    assert_eq!(error.error_code(), 8200);
    release_reservation(&db, &old).await;
    release_reservation(&db, &old).await;
    assert_accounting(&db, &code, 0, 1).await;
    commit_signup(&db, &next, &person("next@example.test"))
        .await
        .unwrap();
    assert_accounting(&db, &code, 1, 0).await;
    db.drop().await.unwrap();
}

async fn held_transaction_race(commit: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let reclaim_started = Arc::new(Notify::new());
    let listen = Arc::new(AtomicBool::new(false));
    let (observed, armed) = (reclaim_started.clone(), listen.clone());
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "invite_commit_race",
        mongodb::event::EventHandler::callback(move |event| {
            if let mongodb::event::command::CommandEvent::Started(e) = event
                && e.command_name == "findAndModify"
                && armed.load(Ordering::SeqCst)
            {
                observed.notify_one();
            }
        }),
    )
    .await;
    let code = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let old = reserve_invite_code(&db, &code.code, "slow@example.test")
        .await
        .unwrap();
    expire(&db, &code.id).await;
    let mut session = db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await
        .unwrap();
    commit_signup_in_session(&db, &mut session, &old, &person("slow@example.test"))
        .await
        .unwrap();
    listen.store(true, Ordering::SeqCst);
    let ((), next) = tokio::join!(
        async {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                reclaim_started.notified(),
            )
            .await
            .unwrap();
            if commit {
                session.commit_transaction().await.unwrap();
            } else {
                session.abort_transaction().await.unwrap();
            }
        },
        reserve_invite_code(&db, &code.code, "next@example.test")
    );
    if commit {
        assert!(matches!(next, Err(AppError::InviteCodeExhausted)));
        assert_accounting(&db, &code, 1, 0).await;
    } else {
        let next = next.unwrap();
        assert!(matches!(
            commit_signup(&db, &old, &person("slow@example.test")).await,
            Err(AppError::InviteCodeInvalid)
        ));
        release_reservation(&db, &old).await;
        assert_accounting(&db, &code, 0, 1).await;
        commit_signup(&db, &next, &person("next@example.test"))
            .await
            .unwrap();
        assert_accounting(&db, &code, 1, 0).await;
    }
    db.drop().await.unwrap();
}
#[tokio::test]
async fn reclaim_waiting_on_final_commit_cannot_also_win() {
    held_transaction_race(true).await;
}
#[tokio::test]
async fn reclaim_waiting_on_aborted_account_transaction_wins_once() {
    held_transaction_race(false).await;
}

#[tokio::test]
async fn simultaneous_expired_commit_and_reclaim_have_one_winner() {
    let db = database("invite_simultaneous").await;
    let code = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let old = reserve_invite_code(&db, &code.code, "slow@example.test")
        .await
        .unwrap();
    expire(&db, &code.id).await;
    let user = person("slow@example.test");
    let barrier = tokio::sync::Barrier::new(2);
    let (commit, next) = tokio::join!(
        async {
            barrier.wait().await;
            commit_signup(&db, &old, &user).await
        },
        async {
            barrier.wait().await;
            reserve_invite_code(&db, &code.code, "next@example.test").await
        }
    );
    match (commit, next) {
        (Ok(()), Err(AppError::InviteCodeExhausted)) => assert_accounting(&db, &code, 1, 0).await,
        (Err(AppError::InviteCodeInvalid), Ok(next)) => {
            assert_accounting(&db, &code, 0, 1).await;
            commit_signup(&db, &next, &person("next@example.test"))
                .await
                .unwrap();
            assert_accounting(&db, &code, 1, 0).await;
        }
        result => panic!("unexpected race result: {result:?}"),
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn failed_user_insert_rolls_back_redemption_then_release_is_idempotent() {
    let db = database("invite_insert_failure").await;
    let code = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let reservation = reserve_invite_code(&db, &code.code, "failed@example.test")
        .await
        .unwrap();
    db.create_collection(USERS_COLLECTION).await.unwrap();
    db.run_command(doc! {"collMod":USERS_COLLECTION,"validator":{"email":{"$ne":"failed@example.test"}},"validationLevel":"strict"}).await.unwrap();
    assert!(
        commit_signup(&db, &reservation, &person("failed@example.test"))
            .await
            .is_err()
    );
    assert_accounting(&db, &code, 0, 1).await;
    let ((), ()) = tokio::join!(
        release_reservation(&db, &reservation),
        release_reservation(&db, &reservation)
    );
    assert_accounting(&db, &code, 0, 0).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_email_is_single_use_and_reclaim_fences_the_old_identity() {
    let db = database("invite_pending_email").await;
    let code = create_invite_code(&db, "admin", 3, None).await.unwrap();
    let old = reserve_invite_code(&db, &code.code, "same@example.test")
        .await
        .unwrap();
    assert!(matches!(
        reserve_invite_code(&db, &code.code, " Same@Example.TEST ").await,
        Err(AppError::InviteCodeAlreadyRedeemed)
    ));
    expire(&db, &code.id).await;
    let next = reserve_invite_code(&db, &code.code, "same@example.test")
        .await
        .unwrap();
    assert_ne!(old.reservation_id, next.reservation_id);
    release_reservation(&db, &old).await;
    assert!(matches!(
        commit_signup(&db, &old, &person("same@example.test")).await,
        Err(AppError::InviteCodeInvalid)
    ));
    commit_signup(&db, &next, &person("same@example.test"))
        .await
        .unwrap();
    assert_accounting(&db, &code, 1, 0).await;
    db.drop().await.unwrap();
}

async fn legacy_reserve(db: &Database, code: &str, email: &str) {
    let result=db.collection::<Document>(COLLECTION_NAME).find_one_and_update(
        doc!{"code":code,"is_active":true,"$expr":{"$lt":["$used_count","$max_uses"]},"usages.email":{"$ne":email}},
        doc!{"$inc":{"used_count":1}},
    ).await.unwrap();
    assert!(result.is_some());
}
async fn legacy_release(db: &Database, id: &str) {
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":id,"used_count":{"$gt":0}},
            doc! {"$inc":{"used_count":-1}},
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn legacy_release_and_new_reclaim_preserve_each_others_capacity_in_both_orders() {
    for release_first in [false, true] {
        for legacy_first in [false, true] {
            let db = database("invite_mixed_release").await;
            let code = create_invite_code(&db, "admin", 3, None).await.unwrap();
            if legacy_first {
                legacy_reserve(&db, &code.code, "legacy@example.test").await;
            }
            let old = reserve_invite_code(&db, &code.code, "new@example.test")
                .await
                .unwrap();
            if !legacy_first {
                legacy_reserve(&db, &code.code, "legacy@example.test").await;
            }
            expire(&db, &code.id).await;
            if release_first {
                legacy_release(&db, &code.id).await;
            }
            let next = reserve_invite_code(&db, &code.code, "next@example.test")
                .await
                .unwrap();
            if !release_first {
                legacy_release(&db, &code.id).await;
            }
            release_reservation(&db, &old).await;
            assert_accounting(&db, &code, 0, 1).await;
            commit_signup(&db, &next, &person("next@example.test"))
                .await
                .unwrap();
            assert_accounting(&db, &code, 1, 0).await;
            db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn legacy_rows_and_in_flight_legacy_increments_are_never_reclaimed() {
    let db = database("invite_legacy_committed").await;
    let code = create_invite_code(&db, "admin", 3, None).await.unwrap();
    db.collection::<Document>(COLLECTION_NAME).update_one(doc!{"_id":&code.id},doc!{"$unset":{"reservations":""},"$set":{"used_count":1},"$push":{"usages":{"user_id":"legacy","used_at":bson::DateTime::now()}}}).await.unwrap();
    legacy_reserve(&db, &code.code, "old-in-flight@example.test").await;
    let abandoned = reserve_invite_code(&db, &code.code, "abandoned@example.test")
        .await
        .unwrap();
    expire(&db, &code.id).await;
    let next = reserve_invite_code(&db, &code.code, "next@example.test")
        .await
        .unwrap();
    release_reservation(&db, &abandoned).await;
    let stored = saved(&db, &code.id).await;
    assert_eq!(stored.used_count, 3); // committed legacy + pending legacy + new reservation
    assert_eq!(stored.usages.len(), 1);
    assert_eq!(stored.reservations.len(), 1);
    legacy_release(&db, &code.id).await;
    commit_signup(&db, &next, &person("next@example.test"))
        .await
        .unwrap();
    let stored = saved(&db, &code.id).await;
    assert_eq!(stored.used_count, 2);
    assert_eq!(stored.usages.len(), 2);
    assert!(stored.reservations.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn social_signup_commits_the_reservation_and_reclaimed_signup_creates_nothing() {
    use crate::services::social_auth_service::{
        SocialProfile, SocialProvider, find_or_create_user_with_invite,
    };
    let db = database("invite_social_commit").await;
    let code = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let profile = SocialProfile {
        provider: SocialProvider::Google,
        provider_id: "social-one".into(),
        email: "social@example.test".into(),
        display_name: None,
        avatar_url: None,
    };
    let old = reserve_invite_code(&db, &code.code, &profile.email)
        .await
        .unwrap();
    expire(&db, &code.id).await;
    let next = reserve_invite_code(&db, &code.code, &profile.email)
        .await
        .unwrap();
    assert!(matches!(
        find_or_create_user_with_invite(&db, &profile, true, Some(&old)).await,
        Err(AppError::InviteCodeInvalid)
    ));
    assert_accounting(&db, &code, 0, 1).await;
    let created = find_or_create_user_with_invite(&db, &profile, true, Some(&next))
        .await
        .unwrap();
    assert!(created.was_newly_created);
    assert_eq!(
        created.user.invite_code_id.as_deref(),
        Some(code.id.as_str())
    );
    release_reservation(&db, &next).await;
    assert_accounting(&db, &code, 1, 0).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_completion_and_new_reclaim_preserve_both_committed_users() {
    for legacy_commits_first in [false, true] {
        let db = database("invite_mixed_commit").await;
        let code = create_invite_code(&db, "admin", 2, None).await.unwrap();
        legacy_reserve(&db, &code.code, "legacy@example.test").await;
        let old = reserve_invite_code(&db, &code.code, "abandoned@example.test")
            .await
            .unwrap();
        let legacy = person("legacy@example.test");
        let complete_legacy = || async {
            db.collection::<crate::models::user::User>(USERS_COLLECTION)
                .insert_one(&legacy)
                .await
                .unwrap();
            db.collection::<Document>(COLLECTION_NAME).update_one(doc!{"_id":&code.id},doc!{"$push":{"usages":{"user_id":&legacy.id,"email":&legacy.email,"used_at":bson::DateTime::now()}}}).await.unwrap();
        };
        if legacy_commits_first {
            complete_legacy().await;
        }
        expire(&db, &code.id).await;
        let next = reserve_invite_code(&db, &code.code, "next@example.test")
            .await
            .unwrap();
        if !legacy_commits_first {
            complete_legacy().await;
        }
        release_reservation(&db, &old).await;
        assert_accounting(&db, &code, 1, 1).await;
        commit_signup(&db, &next, &person("next@example.test"))
            .await
            .unwrap();
        assert_accounting(&db, &code, 2, 0).await;
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn reservation_is_bound_to_the_email_and_inactive_codes_stay_inactive() {
    let db = database("invite_email_binding").await;
    let code = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let reservation = reserve_invite_code(&db, &code.code, "right@example.test")
        .await
        .unwrap();
    assert!(matches!(
        commit_signup(&db, &reservation, &person("wrong@example.test")).await,
        Err(AppError::InviteCodeInvalid)
    ));
    assert_accounting(&db, &code, 0, 1).await;
    deactivate_invite_code(&db, &code.id).await.unwrap();
    expire(&db, &code.id).await;
    assert!(matches!(
        reserve_invite_code(&db, &code.code, "right@example.test").await,
        Err(AppError::InviteCodeDeactivated)
    ));
    // A prior reservation is still admitted, matching the old signup boundary.
    commit_signup(&db, &reservation, &person("right@example.test"))
        .await
        .unwrap();
    assert_accounting(&db, &code, 1, 0).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn password_handler_keeps_default_gate_and_public_signup_does_not_touch_invites() {
    use crate::handlers::auth::{RegisterRequest, register};
    use axum::{
        Json,
        extract::{ConnectInfo, State},
        http::HeaderMap,
    };
    let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
    let observed = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "invite_handler",
        mongodb::event::EventHandler::callback(move |event| {
            if let mongodb::event::command::CommandEvent::Started(e) = event {
                observed.lock().unwrap().push(e.command);
            }
        }),
    )
    .await;
    let mut config = crate::test_utils::test_app_config();
    config.email_auth_enabled = true;
    let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
    let peer = ConnectInfo("127.0.0.1:1234".parse().unwrap());
    let request = |email: &str, code: Option<String>| {
        Json(RegisterRequest {
            email: email.into(),
            password: crate::crypto::token::generate_random_token(),
            display_name: None,
            invite_code: code,
        })
    };
    let missing = register(
        State(state.clone()),
        peer,
        HeaderMap::new(),
        request("a@example.test", None),
    )
    .await;
    assert!(matches!(missing, Err(AppError::ValidationError(_))));
    let code = create_invite_code(&db, "admin", 2, None).await.unwrap();
    let result = register(
        State(state.clone()),
        peer,
        HeaderMap::new(),
        request("a@example.test", Some(code.code.clone())),
    )
    .await
    .unwrap();
    assert!(!result.user_id.is_empty());
    assert_accounting(&db, &code, 1, 0).await;
    // A second invite for an existing email returns fake success and refunds its reservation.
    let duplicate = create_invite_code(&db, "admin", 1, None).await.unwrap();
    let _ = register(
        State(state.clone()),
        peer,
        HeaderMap::new(),
        request("a@example.test", Some(duplicate.code.clone())),
    )
    .await
    .unwrap();
    assert_eq!(saved(&db, &duplicate.id).await.used_count, 0);
    assert!(saved(&db, &duplicate.id).await.reservations.is_empty());
    db.collection::<Document>("feature_flag_overrides").insert_one(doc!{"_id":Uuid::new_v4().to_string(),"flag_key":"auth:invitation-code","target_kind":"global","updated_by":"admin","enabled":false,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    commands.lock().unwrap().clear();
    let _ = register(
        State(state.clone()),
        peer,
        HeaderMap::new(),
        request("public@example.test", Some("ignored-invalid".into())),
    )
    .await
    .unwrap();
    let invite_commands = commands
        .lock()
        .unwrap()
        .iter()
        .filter(|d| d.values().any(|v| v.as_str() == Some(COLLECTION_NAME)))
        .count();
    assert_eq!(
        invite_commands, 0,
        "public registration must not access invitation state"
    );
    // Audit appends are detached; this fixture is deleted with its container.
}
