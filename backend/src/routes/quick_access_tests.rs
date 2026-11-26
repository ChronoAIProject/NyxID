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
#[tokio::test]
async fn p1_5_quick_access_production_router_credential_matrix() {
    use crate::services::{key_service, service_account_service};
    use mongodb::bson::doc;
    let db = connect_test_database("quick_access_credentials_router")
        .await
        .expect("Mongo required");
    let person = Uuid::new_v4();
    let owner = person.to_string();
    db.collection::<User>(COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let access = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &person,
        "openid account:read",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let session = token_service::create_session(&db, &owner, None, None)
        .await
        .unwrap();
    let key = key_service::create_api_key(
        &db,
        &owner,
        "quick access fixture",
        "read proxy",
        None,
        None,
        None,
        None,
        Some(true),
        None,
        Some(true),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let raw_key = key.full_key;
    let key_id = key.id;
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &person,
        "account:read",
        "quick-access-client",
        60,
        None,
    )
    .unwrap();
    let relay = jwt::generate_relay_access_token(
        &state.jwt_keys,
        &state.config,
        &person,
        "proxy",
        None,
        &jwt::RelayAgentScope {
            api_key_id: key_id,
            api_key_name: "quick access fixture".into(),
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
        },
    )
    .unwrap();
    let (oauth_client, _) = crate::services::oauth_client_service::create_client(
        &db,
        "quick access OAuth fixture",
        &["https://example.test/callback".into()],
        "public",
        &owner,
        "",
        "account:read",
        crate::models::oauth_client::ScopeProvenance::Explicit,
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap();
    let consent = crate::services::consent_service::grant_consent(
        &db,
        &owner,
        &oauth_client.id,
        "account:read",
    )
    .await
    .unwrap();
    let reference = crate::services::consent_service::protected_reference(&consent).unwrap();
    let oauth = jwt::generate_oauth_access_token_with_consent(
        &state.jwt_keys,
        &state.config,
        &person,
        "account:read",
        None,
        None,
        None,
        None,
        None,
        &oauth_client.id,
        Some(&reference),
    )
    .unwrap();
    let (service, secret) = service_account_service::create_service_account(
        &db,
        "quick access fixture",
        None,
        "account:read",
        &[],
        None,
        &owner,
    )
    .await
    .unwrap();
    let service_token = service_account_service::authenticate_client_credentials(
        &db,
        &state.config,
        &state.jwt_keys,
        &service.client_id,
        &secret,
        Some("account:read"),
    )
    .await
    .unwrap()
    .access_token;
    let (_, private) = build_router_with_state(state.clone());
    let app = private.with_state(state);
    for (kind, header, value, allowed) in [
        (
            "access JWT",
            "authorization",
            format!("Bearer {access}"),
            true,
        ),
        (
            "session cookie",
            "cookie",
            format!("nyx_session={}", session.session_token),
            true,
        ),
        ("API key", "x-api-key", raw_key, false),
        (
            "delegated account:read",
            "authorization",
            format!("Bearer {delegated}"),
            false,
        ),
        ("relay", "authorization", format!("Bearer {relay}"), false),
        (
            "OAuth-client",
            "authorization",
            format!("Bearer {oauth}"),
            false,
        ),
        (
            "service-account",
            "authorization",
            format!("Bearer {service_token}"),
            false,
        ),
        ("anonymous", "authorization", "".into(), false),
    ] {
        if kind != "anonymous" {
            let path = if kind == "relay" {
                "/api/v1/llm/status"
            } else {
                "/api/v1/channel-platforms"
            };
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header(header, &value)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "N8 {kind} positive control {path}"
            );
        }
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
            if allowed {
                assert_eq!(response.status(), StatusCode::OK, "{kind} {method}");
            } else {
                assert_eq!(
                    response.status(),
                    if kind == "anonymous" {
                        StatusCode::UNAUTHORIZED
                    } else {
                        StatusCode::FORBIDDEN
                    },
                    "N8 {kind} {method} policy refusal"
                );
            }
        }
        let row = db
            .collection::<User>(COLLECTION_NAME)
            .find_one(doc! {"_id": &owner})
            .await
            .unwrap()
            .unwrap();
        assert!(
            row.profile_config.quick_access.is_none(),
            "{kind} changed stored preference"
        );
    }
    db.drop().await.unwrap();
}
