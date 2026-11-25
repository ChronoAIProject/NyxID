use std::cell::Cell;

use axum::{http::StatusCode, response::IntoResponse};
use mongodb::bson::{DateTime, doc};

use crate::{
    models::user::UserType,
    services::{
        auth_service,
        coordination_service::TEST_RATE_WINDOW_NOW,
        social_auth_service::{self, SocialProfile, SocialProvider},
    },
    test_utils::{connect_transaction_test_database, drop_test_database, test_user},
};

use super::*;

fn user(db: &Database) -> mongodb::Collection<User> {
    db.collection::<User>(USERS)
}

fn new_person() -> User {
    test_user(&uuid::Uuid::new_v4().to_string(), UserType::Person)
}

fn net(limits: SignupLimits, octet: u8) -> SignupAdmission {
    SignupAdmission::new(limits, std::net::Ipv4Addr::new(192, 0, 2, octet).into())
}

fn at(day: u32, hour: u32) -> DateTime {
    use chrono::TimeZone;
    DateTime::from_chrono(
        chrono::Utc
            .with_ymd_and_hms(2026, 1, day, hour, 30, 0)
            .unwrap(),
    )
}

#[tokio::test]
async fn signup_admission_enforces_all_windows_before_inserting_a_person() {
    let db = connect_transaction_test_database("signup_windows").await;
    let limits = SignupLimits::for_test(2, 3, 4);
    let first = net(limits, 1);
    let second = net(limits, 2);
    let third = net(limits, 3);

    TEST_RATE_WINDOW_NOW
        .scope(Cell::new(at(10, 10)), async {
            first.insert_user(&db, &new_person()).await.unwrap();
            first.insert_user(&db, &new_person()).await.unwrap();
            let refused = new_person();
            let error = first.insert_user(&db, &refused).await.unwrap_err();
            assert_eq!(error.error_key(), "rate_limited");
            assert_eq!(
                error.into_response().status(),
                StatusCode::TOO_MANY_REQUESTS
            );
            assert!(
                user(&db)
                    .find_one(doc! {"_id": &refused.id})
                    .await
                    .unwrap()
                    .is_none()
            );

            TEST_RATE_WINDOW_NOW.with(|now| now.set(at(10, 11)));
            first.insert_user(&db, &new_person()).await.unwrap();
            assert!(matches!(
                first.insert_user(&db, &new_person()).await,
                Err(AppError::RateLimited)
            ));
            second.insert_user(&db, &new_person()).await.unwrap();
            assert!(matches!(
                third.insert_user(&db, &new_person()).await,
                Err(AppError::RateLimited)
            ));
            assert_eq!(user(&db).count_documents(doc! {}).await.unwrap(), 4);

            TEST_RATE_WINDOW_NOW.with(|now| now.set(at(11, 0)));
            first.insert_user(&db, &new_person()).await.unwrap();
            third.insert_user(&db, &new_person()).await.unwrap();
            assert_eq!(user(&db).count_documents(doc! {}).await.unwrap(), 6);
        })
        .await;
    drop_test_database(&db).await;
}

#[tokio::test]
async fn signup_admission_does_not_spend_capacity_on_rolled_back_writes() {
    let db = connect_transaction_test_database("signup_rollback").await;
    let limits = SignupLimits::for_test(2, 2, 2);
    let admission = net(limits, 1);
    TEST_RATE_WINDOW_NOW
        .scope(Cell::new(at(10, 10)), async {
            let person = new_person();
            admission.insert_user(&db, &person).await.unwrap();
            assert!(admission.insert_user(&db, &person).await.is_err());
            admission.insert_user(&db, &new_person()).await.unwrap();
            assert!(matches!(
                admission.insert_user(&db, &new_person()).await,
                Err(AppError::RateLimited)
            ));
            assert_eq!(user(&db).count_documents(doc! {}).await.unwrap(), 2);
        })
        .await;
    drop_test_database(&db).await;
}

#[tokio::test]
async fn signup_admission_serializes_concurrent_writers() {
    let db = connect_transaction_test_database("signup_race").await;
    let limits = SignupLimits::for_test(5, 20, 500);
    let admission = net(limits, 1);
    let results = futures::future::join_all((0..10).map(|_| {
        let db = db.clone();
        async move {
            TEST_RATE_WINDOW_NOW
                .scope(Cell::new(at(10, 10)), async {
                    admission.insert_user(&db, &new_person()).await
                })
                .await
        }
    }))
    .await;
    let mut created = 0;
    for result in results {
        match result {
            Ok(()) => created += 1,
            Err(AppError::RateLimited) => {}
            Err(error) => panic!("unexpected signup result: {error}"),
        }
    }
    assert_eq!(created, 5);
    assert_eq!(user(&db).count_documents(doc! {}).await.unwrap(), 5);
    drop_test_database(&db).await;
}

#[tokio::test]
async fn signup_admission_shares_email_and_social_caps_and_allows_existing_sign_in() {
    let db = connect_transaction_test_database("signup_shared").await;
    let admission = net(SignupLimits::for_test(2, 3, 4), 1);
    let email = "email-signup@example.test";
    let mut social = SocialProfile {
        provider: SocialProvider::GitHub,
        provider_id: "synthetic-id".to_string(),
        email: "social-signup@example.test".to_string(),
        display_name: None,
        avatar_url: None,
    };
    TEST_RATE_WINDOW_NOW
        .scope(Cell::new(at(10, 10)), async {
            let email_result = auth_service::register_public_user(
                &db,
                email,
                "synthetic-password",
                None,
                None,
                true,
                Some(admission),
            )
            .await
            .unwrap();
            assert!(email_result.actually_created);
            let social_result = social_auth_service::find_or_create_public_user(
                &db,
                &social,
                true,
                None,
                Some(admission),
            )
            .await
            .unwrap();
            assert!(social_result.was_newly_created);
            let returning = social_auth_service::find_or_create_public_user(
                &db,
                &social,
                true,
                None,
                Some(admission),
            )
            .await
            .unwrap();
            assert_eq!(returning.user.id, social_result.user.id);
            assert!(!returning.was_newly_created);
            for email in [email, "another-signup@example.test"] {
                assert!(matches!(
                    auth_service::register_public_user(
                        &db,
                        email,
                        "synthetic-password",
                        None,
                        None,
                        true,
                        Some(admission),
                    )
                    .await,
                    Err(AppError::RateLimited)
                ));
            }
            social.provider_id = "another-synthetic-id".to_string();
            social.email = "other-social@example.test".to_string();
            assert!(matches!(
                social_auth_service::find_or_create_public_user(
                    &db,
                    &social,
                    true,
                    None,
                    Some(admission),
                )
                .await,
                Err(AppError::RateLimited)
            ));
            assert_eq!(user(&db).count_documents(doc! {}).await.unwrap(), 2);
        })
        .await;
    drop_test_database(&db).await;
}
