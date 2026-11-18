use super::*;
use crate::{
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::{role_service, security_readiness_service},
    test_utils,
};
use bson::doc;

async fn setup() -> (AppState, AuthUser, AuthUser, AuthUser) {
    let db = test_utils::connect_transaction_test_database("platform_model_credentials").await;
    test_utils::init_test_audit_chain_hmac_key();
    role_service::seed_system_roles(&db).await.unwrap();
    security_readiness_service::ready_test_replica(&db).await;
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let mut auths = Vec::new();
    for role in [Some(roles.admin), Some(roles.operator), None] {
        let id = uuid::Uuid::new_v4().to_string();
        let mut row = test_utils::test_user(&id, UserType::Person);
        if let Some(role) = role {
            row.role_ids.push(role);
        }
        db.collection::<User>(USERS).insert_one(row).await.unwrap();
        auths.push(test_utils::test_auth_user(&id));
    }
    db.collection::<bson::Document>(crate::models::feature_flag_override::COLLECTION_NAME).insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,"flag_key":super::super::registry::FLAG,"target_kind":"global","target_key":bson::Bson::Null,"enabled":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"updated_by":auths[0].user_id.to_string()}).await.unwrap();
    (
        test_utils::test_app_state(db),
        auths.remove(0),
        auths.remove(0),
        auths.remove(0),
    )
}
fn input() -> CreateCredential {
    CreateCredential {
        name: "Test OpenAI".into(),
        kind: Kind::Openai,
        credential: zeroize::Zeroizing::new("test-private-provider-key".into()),
    }
}
async fn add(state: &AppState, auth: &AuthUser) -> Credential {
    create(
        State(state.clone()),
        auth.clone(),
        TelemetryContext::default(),
        Json(input()),
    )
    .await
    .unwrap()
    .0
}
#[tokio::test]
async fn credentials_and_offerings_use_closed_catalog_shapes_and_safe_operator_projection() {
    let (state, admin, operator, person) = setup().await;
    for auth in [&operator, &person] {
        assert!(
            create(
                State(state.clone()),
                auth.clone(),
                TelemetryContext::default(),
                Json(input())
            )
            .await
            .is_err()
        );
    }
    let mut key = admin.clone();
    key.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    assert!(
        create(
            State(state.clone()),
            key.clone(),
            TelemetryContext::default(),
            Json(input())
        )
        .await
        .is_err()
    );
    let row = add(&state, &admin).await;
    let stored = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.credential_encrypted.is_empty());
    assert_eq!(stored.visibility, "private");
    assert_eq!(stored.auth_method, "bearer");
    assert!(!stored.requires_user_credential);
    assert!(
        stored
            .platform_key
            .as_ref()
            .unwrap()
            .allowed_owner_ids
            .is_empty()
    );
    assert!(
        !crate::services::platform_key_service::available(
            &state.db,
            &stored,
            &person.user_id.to_string()
        )
        .await
        .unwrap()
    );
    let view = list(
        State(state.clone()),
        operator.clone(),
        Query(ListQuery::default()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(view.records.len(), 1);
    let json = serde_json::to_string(&view).unwrap();
    for secret in [
        "test-private-provider-key",
        "credential_encrypted",
        "fingerprint",
        "private_key",
        "client_email",
    ] {
        assert!(!json.contains(secret));
    }
    assert!(
        list(State(state.clone()), person, Query(ListQuery::default()))
            .await
            .is_err()
    );
    assert!(
        list(State(state.clone()), key, Query(ListQuery::default()))
            .await
            .is_err()
    );
    let offering = create_offering(
        State(state.clone()),
        admin.clone(),
        TelemetryContext::default(),
        Json(CreateOffering {
            name: "Billing identity".into(),
        }),
    )
    .await
    .unwrap()
    .0;
    let stored = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&offering.id})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.credential_encrypted.is_empty());
    assert_eq!(stored.auth_method, "none");
    assert_eq!(
        offerings(State(state.clone()), operator, Query(ListQuery::default()))
            .await
            .unwrap()
            .0
            .records
            .len(),
        1
    );
    assert_eq!(
        list(
            State(state.clone()),
            admin.clone(),
            Query(ListQuery::default())
        )
        .await
        .unwrap()
        .0
        .records
        .len(),
        1
    );
    state
        .db
        .collection::<bson::Document>(crate::models::feature_flag_override::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set":{"enabled":false}})
        .await
        .unwrap();
    assert!(
        create(
            State(state.clone()),
            admin.clone(),
            TelemetryContext::default(),
            Json(input())
        )
        .await
        .is_err()
    );
    assert!(
        list(State(state.clone()), admin, Query(ListQuery::default()))
            .await
            .is_ok()
    );
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn rotate_is_revision_fenced_and_disable_preserves_secret_without_exposing_it() {
    let (state, admin, _, _) = setup().await;
    let row = add(&state, &admin).await;
    let revision = chrono::DateTime::parse_from_rfc3339(&row.updated_at)
        .unwrap()
        .with_timezone(&chrono::Utc);
    let patch = || UpdateCredential {
        expected_updated_at: revision,
        credential: Some(zeroize::Zeroizing::new("replacement-test-key".into())),
        enabled: None,
    };
    let updated = update(
        State(state.clone()),
        admin.clone(),
        TelemetryContext::default(),
        Path(row.id.clone()),
        Json(patch()),
    )
    .await
    .unwrap()
    .0;
    assert_ne!(updated.updated_at, row.updated_at);
    assert!(matches!(
        update(
            State(state.clone()),
            admin.clone(),
            TelemetryContext::default(),
            Path(row.id.clone()),
            Json(patch())
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let disabled = update(
        State(state.clone()),
        admin.clone(),
        TelemetryContext::default(),
        Path(row.id.clone()),
        Json(UpdateCredential {
            expected_updated_at: chrono::DateTime::parse_from_rfc3339(&updated.updated_at)
                .unwrap()
                .with_timezone(&chrono::Utc),
            credential: None,
            enabled: Some(false),
        }),
    )
    .await
    .unwrap()
    .0;
    assert!(!disabled.enabled);
    assert_eq!(disabled.id, row.id);
    let stored = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.credential_encrypted.is_empty());
    let material =
        crate::services::catalog_credential_record_service::text(&state.encryption_keys, &stored)
            .await
            .unwrap();
    assert!(material.as_str() == "replacement-test-key");
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn invalid_service_account_fails_save_without_a_catalog_row_or_secret_in_error() {
    let (state, admin, _, _) = setup().await;
    for raw in [
        "not-json-private-material",
        r#"{"type":"external_account","credential_source":{"file":"private-file"}}"#,
        r#"{"type":"service_account","private_key":"private-pem"}"#,
    ] {
        let error = create(
            State(state.clone()),
            admin.clone(),
            TelemetryContext::default(),
            Json(CreateCredential {
                name: "Invalid".into(),
                kind: Kind::GcpServiceAccount,
                credential: zeroize::Zeroizing::new(raw.into()),
            }),
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(error, AppError::ValidationError(_)));
        assert!(!error.to_string().contains("private-"));
    }
    assert_eq!(
        state
            .db
            .collection::<DownstreamService>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn only_current_route_references_block_disable_and_inventory_is_cross_creator() {
    let (state, admin, operator, _) = setup().await;
    let row = add(&state, &admin).await;
    use crate::models::platform_model::{COLLECTION_NAME as MODELS, ROUTING_COLLECTION};
    let model = uuid::Uuid::new_v4().to_string();
    let revision = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>(MODELS).insert_one(doc!{"_id":&model,"public_id":"nyxid/test-model","display_name":"Test model","routing_revision_id":&revision,"enabled":false}).await.unwrap();
    state.db.collection::<bson::Document>(ROUTING_COLLECTION).insert_one(doc!{"_id":&revision,"model_id":&model,"routes":[{"id":"test-route","credential_service_id":&row.id,"enabled":true}]}).await.unwrap();
    let listed = list(State(state.clone()), operator, Query(ListQuery::default()))
        .await
        .unwrap()
        .0;
    assert_eq!(
        listed.records[0].references[0].public_id,
        "nyxid/test-model"
    );
    let patch = || UpdateCredential {
        expected_updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)
            .unwrap()
            .with_timezone(&chrono::Utc),
        credential: None,
        enabled: Some(false),
    };
    assert!(matches!(
        update(
            State(state.clone()),
            admin.clone(),
            TelemetryContext::default(),
            Path(row.id.clone()),
            Json(patch())
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    state
        .db
        .collection::<bson::Document>(MODELS)
        .update_one(
            doc! {"_id":&model},
            doc! {"$set":{"routing_revision_id":"new-revision"}},
        )
        .await
        .unwrap();
    assert!(
        !update(
            State(state.clone()),
            admin,
            TelemetryContext::default(),
            Path(row.id),
            Json(patch())
        )
        .await
        .unwrap()
        .0
        .enabled
    );
    state.db.drop().await.unwrap();
}

#[test]
fn malformed_credential_request_fields_never_echo_secret_values() {
    for value in [
        serde_json::json!({"name":"n","kind":"secret-material","credential":"key"}),
        serde_json::json!({"name":"n","kind":"openai","credential":1234567890}),
        serde_json::json!({"name":"n","kind":"openai","credential":"key","secret-material":"x"}),
    ] {
        let result = decode_secret::<CreateCredential>(Ok(Json(
            crate::crypto::secret_json::SecretJson(value),
        )));
        assert!(
            matches!(result,Err(AppError::ValidationError(ref message)) if message == "Invalid credential request fields")
        );
    }
}
#[tokio::test]
async fn simultaneous_replacements_from_one_revision_commit_only_once() {
    let (state, admin, _, _) = setup().await;
    let row = add(&state, &admin).await;
    let revision = chrono::DateTime::parse_from_rfc3339(&row.updated_at)
        .unwrap()
        .with_timezone(&chrono::Utc);
    let make = |value: &str| UpdateCredential {
        expected_updated_at: revision,
        credential: Some(zeroize::Zeroizing::new(value.into())),
        enabled: None,
    };
    let (a, b) = tokio::join!(
        update(
            State(state.clone()),
            admin.clone(),
            TelemetryContext::default(),
            Path(row.id.clone()),
            Json(make("test-key-one"))
        ),
        update(
            State(state.clone()),
            admin,
            TelemetryContext::default(),
            Path(row.id.clone()),
            Json(make("test-key-two"))
        )
    );
    assert_eq!([&a, &b].iter().filter(|r| r.is_ok()).count(), 1);
    assert!(
        [a, b]
            .iter()
            .any(|r| matches!(r, Err(AppError::Conflict(_))))
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn all_supported_credential_kinds_save_encrypted_without_public_metadata_from_the_key() {
    let (state, admin, operator, _) = setup().await;
    let gcp=serde_json::json!({"type":"service_account","project_id":"private-project","private_key_id":"fixture-id","private_key":test_utils::TEST_GCP_SA_PRIVATE_KEY,"client_email":"fixture@private-project.iam.gserviceaccount.com","client_id":"fixture-client","token_uri":"https://oauth2.googleapis.com/token"}).to_string();
    for (kind, raw) in [
        (Kind::GcpServiceAccount, gcp.as_str()),
        (Kind::Anthropic, "test-anthropic-key"),
    ] {
        let body = serde_json::json!({"name":"Provider","kind":kind,"credential":raw});
        let row = create_request(
            State(state.clone()),
            admin.clone(),
            TelemetryContext::default(),
            Ok(Json(crate::crypto::secret_json::SecretJson(body))),
        )
        .await
        .unwrap()
        .0;
        let response = get(State(state.clone()), operator.clone(), Path(row.id.clone()))
            .await
            .unwrap()
            .0;
        let json = serde_json::to_string(&response).unwrap();
        for hidden in [
            "private-project",
            "fixture-id",
            "fixture-client",
            "test-anthropic-key",
            "PRIVATE KEY",
        ] {
            assert!(!json.contains(hidden));
        }
        let stored = state
            .db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.credential_encrypted.is_empty());
        assert!(credentials::Kind::from_url(&stored.base_url) == Some(kind));
    }
    state.db.drop().await.unwrap();
}
