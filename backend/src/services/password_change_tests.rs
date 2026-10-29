use super::*;
use crate::test_utils::{connect_test_database, test_user};

async fn fixture(prefix: &str) -> (mongodb::Database, User, String) {
    let db = connect_test_database(prefix).await.expect("test MongoDB");
    let mut user = test_user(&Uuid::new_v4().to_string(), UserType::Person);
    let current_password = generate_random_token();
    user.password_hash = Some(password::hash_password(&current_password).unwrap());
    user.password_reset_token = Some(hash_token(&generate_random_token()));
    user.password_reset_expires_at = Some(Utc::now() + chrono::Duration::hours(1));
    db.collection::<User>(USERS)
        .insert_one(&user)
        .await
        .unwrap();
    (db, user, current_password)
}

#[tokio::test]
async fn password_change_only_one_concurrent_verified_old_hash_can_win() {
    let (db, user, _) = fixture("password_change_cas").await;
    let old_hash = user.password_hash.as_deref().unwrap();
    let first_hash = password::hash_password(&generate_random_token()).unwrap();
    let second_hash = password::hash_password(&generate_random_token()).unwrap();
    let (first, second) = tokio::join!(
        replace_verified_password(&db, &user.id, old_hash, &first_hash),
        replace_verified_password(&db, &user.id, old_hash, &second_hash),
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let winning_hash = if first.is_ok() {
        &first_hash
    } else {
        &second_hash
    };
    assert!(matches!(
        first.err().or(second.err()),
        Some(AppError::Conflict(_))
    ));
    let stored = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &user.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.password_hash.as_deref(), Some(winning_hash.as_str()));
    assert!(stored.password_reset_token.is_none());
    assert!(stored.password_reset_expires_at.is_none());
}

#[tokio::test]
async fn password_change_and_reset_final_updates_fence_each_other() {
    let (db, user, current_password) = fixture("password_change_reset_race").await;
    let new_password = generate_random_token();
    let reset_password_text = generate_random_token();
    let stale_reset_hash = password::hash_password(&generate_random_token()).unwrap();
    let stale_change_hash = password::hash_password(&generate_random_token()).unwrap();
    // The reset has already read and validated this token when the change wins.
    let reset_hash = user.password_reset_token.as_deref().unwrap();
    change_password(
        &db,
        &user.id,
        Zeroizing::new(current_password),
        Zeroizing::new(new_password.clone()),
    )
    .await
    .unwrap();
    assert!(matches!(
        finish_password_reset(&db, &user.id, reset_hash, &stale_reset_hash).await,
        Err(AppError::BadRequest(_))
    ));
    let stored = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &user.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        password::verify_password(&new_password, stored.password_hash.as_deref().unwrap()).unwrap()
    );

    // Reverse ordering: a reset wins after a change verified the old hash.
    let new_reset = initiate_password_reset(&db, &user.email)
        .await
        .unwrap()
        .unwrap();
    reset_password(&db, &new_reset, &reset_password_text)
        .await
        .unwrap();
    assert!(matches!(
        replace_verified_password(
            &db,
            &user.id,
            stored.password_hash.as_deref().unwrap(),
            &stale_change_hash
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let stored = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &user.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        password::verify_password(
            &reset_password_text,
            stored.password_hash.as_deref().unwrap()
        )
        .unwrap()
    );
}

#[tokio::test]
async fn password_change_final_cas_rejects_inactive_org_and_deleting_accounts() {
    for change in [
        doc! {"is_active": false},
        doc! {"user_type":"org"},
        doc! {"platform_personal_deleting":true},
    ] {
        let (db, user, _) = fixture("password_change_live_actor").await;
        let replacement_hash = password::hash_password(&generate_random_token()).unwrap();
        db.collection::<User>(USERS)
            .update_one(doc! {"_id": &user.id}, doc! {"$set": change})
            .await
            .unwrap();
        assert!(matches!(
            replace_verified_password(
                &db,
                &user.id,
                user.password_hash.as_deref().unwrap(),
                &replacement_hash
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            finish_password_reset(
                &db,
                &user.id,
                user.password_reset_token.as_deref().unwrap(),
                &replacement_hash
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
        let stored = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &user.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.password_hash, user.password_hash);
    }
}

#[tokio::test]
async fn password_reset_final_update_rejects_expired_replaced_and_replayed_tokens() {
    for update in [
        doc! {"password_reset_expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))},
        doc! {"password_reset_token": hash_token(&generate_random_token())},
        doc! {"password_reset_token": null, "password_reset_expires_at": null},
    ] {
        let (db, user, _) = fixture("password_reset_final_fence").await;
        let replacement_hash = password::hash_password(&generate_random_token()).unwrap();
        db.collection::<User>(USERS)
            .update_one(doc! {"_id": &user.id}, doc! {"$set": update})
            .await
            .unwrap();
        assert!(matches!(
            finish_password_reset(
                &db,
                &user.id,
                user.password_reset_token.as_deref().unwrap(),
                &replacement_hash
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
        let stored = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &user.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.password_hash, user.password_hash);
    }
}
