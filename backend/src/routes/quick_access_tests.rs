use super::build_router_with_state;
use crate::{
    crypto::jwt,
    models::user::{COLLECTION_NAME, User, UserType},
    services::token_service,
    test_utils::{connect_test_database, test_app_state, test_user},
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn p2_2_quick_access_refuses_org_subjects_even_with_session_shaped_credentials() {
    let db = connect_test_database("quick_access_org_router")
        .await
        .expect("MongoDB required; must not skip");
    let org = Uuid::new_v4();
    let mut record = test_user(&org.to_string(), UserType::Org);
    record.profile_config.quick_access = Some(crate::models::user::QuickAccessPreferences {
        items: crate::services::user_preferences_service::default_quick_access(),
    });
    let original = record.profile_config.quick_access.clone();
    db.collection::<User>(COLLECTION_NAME)
        .insert_one(record)
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    // Simulate a legacy or erroneously issued credential. Route admission must
    // enforce person identity, independently of normal issuance restrictions.
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &org,
        "openid account:read",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let session = token_service::create_session(&db, &org.to_string(), None, None)
        .await
        .unwrap();
    let (_, private) = build_router_with_state(state.clone());
    let app = private.with_state(state);
    for (header, value) in [
        ("authorization", format!("Bearer {token}")),
        ("cookie", format!("nyx_session={}", session.session_token)),
    ] {
        for method in ["GET", "PUT", "DELETE"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/api/v1/users/me/quick-access")
                        .header(header, &value)
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"items":[]}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(
                matches!(
                    response.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ),
                "org {header} {method}: {}",
                response.status()
            );
        }
    }
    let user = db
        .collection::<User>(COLLECTION_NAME)
        .find_one(mongodb::bson::doc! {"_id":org.to_string()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.profile_config.quick_access, original);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn p2_2_quick_access_preserves_legacy_person_records() {
    use crate::services::user_preferences_service::{
        read_quick_access, reset_quick_access, save_quick_access,
    };
    use mongodb::bson::{Document, doc};
    let db = connect_test_database("quick_access_legacy_person")
        .await
        .expect("Mongo required");
    let id = Uuid::new_v4().to_string();
    let mut row = mongodb::bson::to_document(&test_user(&id, UserType::Person)).unwrap();
    row.remove("user_type");
    db.collection::<Document>(COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
    save_quick_access(&db, &id, vec![]).await.unwrap();
    assert!(read_quick_access(&db, &id).await.unwrap().is_some());
    reset_quick_access(&db, &id).await.unwrap();
    assert!(read_quick_access(&db, &id).await.unwrap().is_none());
    assert!(
        db.collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": &id})
            .await
            .unwrap()
            .is_some()
    );
    db.drop().await.unwrap();
}
