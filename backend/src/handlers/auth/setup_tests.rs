use super::*;
use crate::services::role_service;
use crate::test_utils::{connect_test_database, test_app_state};

async fn request(
    state: &AppState,
    token: Option<&str>,
    email: &str,
) -> AppResult<Json<SetupResponse>> {
    let mut headers = HeaderMap::new();
    if let Some(token) = token {
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
    }
    setup(
        State(state.clone()),
        ConnectInfo(([127, 0, 0, 1], 443).into()),
        headers,
        Json(SetupRequest {
            email: email.to_owned(),
            password: uuid::Uuid::new_v4().to_string(),
            display_name: None,
        }),
    )
    .await
}

#[tokio::test]
async fn initial_admin_requires_configured_matching_secret_and_consumes_it() {
    let db = connect_test_database("initial_admin_secret").await.unwrap();
    let mut state = test_app_state(db.clone());
    role_service::seed_system_roles(&db).await.unwrap();
    let token = uuid::Uuid::new_v4().to_string();
    let email = "operator@example.org";
    assert!(matches!(
        request(&state, Some(&token), email).await,
        Err(AppError::Forbidden(_))
    ));
    state.config.admin_setup_secret_hash = Some(crate::crypto::token::hash_token(&token));
    for supplied in [None, Some("incorrect")] {
        assert!(matches!(
            request(&state, supplied, email).await,
            Err(AppError::Forbidden(_))
        ));
    }
    let users = db.collection::<User>(USERS);
    assert_eq!(users.count_documents(doc! {}).await.unwrap(), 0);
    let id = request(&state, Some(&token), email)
        .await
        .unwrap()
        .0
        .user_id;
    let user = users.find_one(doc! {"_id": &id}).await.unwrap().unwrap();
    assert!(user.email_verified && user.is_admin);
    assert!(
        user.role_ids.contains(
            &role_service::get_platform_role_ids(&db)
                .await
                .unwrap()
                .admin
        )
    );
    assert!(matches!(
        request(&state, Some(&token), email).await,
        Err(AppError::Forbidden(_))
    ));
    users.delete_one(doc! {"_id": id}).await.unwrap();
    assert!(matches!(
        request(&state, Some(&token), email).await,
        Err(AppError::Forbidden(_))
    ));
}

#[tokio::test]
async fn concurrent_initial_admin_requests_create_one_privileged_user() {
    let db = connect_test_database("initial_admin_concurrent")
        .await
        .unwrap();
    let mut state = test_app_state(db.clone());
    role_service::seed_system_roles(&db).await.unwrap();
    let token = uuid::Uuid::new_v4().to_string();
    state.config.admin_setup_secret_hash = Some(crate::crypto::token::hash_token(&token));
    let (first, second) = tokio::join!(
        request(&state, Some(&token), "first@example.org"),
        request(&state, Some(&token), "second@example.org")
    );
    assert!(
        matches!(
            (&first, &second),
            (Ok(_), Err(AppError::Forbidden(_))) | (Err(AppError::Forbidden(_)), Ok(_))
        ),
        "{first:?} {second:?}"
    );
    assert_eq!(
        db.collection::<User>(USERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}
