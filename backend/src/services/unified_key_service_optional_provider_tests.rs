use super::*;
use crate::models::service_provider_requirement::COLLECTION_NAME as REQUIREMENTS;
use crate::services::proxy_service::verify_auto_provision_eligibility;

mod transport_tests {
    include!("unified_key_service_optional_transport_tests.rs");
}

fn no_auth_catalog() -> DownstreamService {
    let mut catalog = sample_catalog_service();
    catalog.id = uuid::Uuid::new_v4().to_string();
    catalog.slug = format!("optional-{}", uuid::Uuid::new_v4());
    catalog.auth_method = "none".into();
    catalog.requires_user_credential = false;
    catalog
}

async fn insert_requirement(db: &mongodb::Database, catalog_id: &str, required: bool) -> String {
    let mut requirement = sample_spr("header", Some("X-GitHub-Token"));
    requirement.id = uuid::Uuid::new_v4().to_string();
    requirement.service_id = catalog_id.into();
    requirement.provider_config_id = uuid::Uuid::new_v4().to_string();
    requirement.required = required;
    db.collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .insert_one(&requirement)
        .await
        .unwrap();
    requirement.id
}

async fn opted_in_service(db: &mongodb::Database, owner: &str, catalog: &str) -> UserService {
    let rows: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! { "user_id": owner, "catalog_service_id": catalog })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "exactly one explicit connection must exist");
    let service = rows.into_iter().next().unwrap();
    assert_eq!(service.source.as_deref(), Some(crate::services::unified_key_service::CATALOG_OPT_IN_SOURCE));
    assert!(service.api_key_id.is_none());
    assert!(service.is_active);
    service
}

async fn assert_endpoint_count(db: &mongodb::Database, owner: &str, expected: u64) {
    assert_eq!(
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .count_documents(doc! { "user_id": owner })
            .await
            .unwrap(),
        expected,
        "automatic provisioning must not leak endpoints"
    );
    assert_eq!(
        db.collection::<UserApiKey>(USER_API_KEYS)
            .count_documents(doc! { "user_id": owner })
            .await
            .unwrap(),
        0,
        "optional providers must not require or create a user credential"
    );
}

#[tokio::test]
async fn optional_only_connects_once_without_credentials() {
    let db = connect_test_database("optional_provision").await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    let catalog = no_auth_catalog();
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    insert_requirement(&db, &catalog.id, false).await;

    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let first = opted_in_service(&db, &owner, &catalog.id).await;
    assert_eq!(first.auth_method, "none");
    assert_endpoint_count(&db, &owner, 1).await;
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let second = opted_in_service(&db, &owner, &catalog.id).await;
    assert_eq!(first.id, second.id);
    assert_eq!(first.endpoint_id, second.endpoint_id);
    assert_endpoint_count(&db, &owner, 1).await;
}

#[tokio::test]
async fn optional_requirement_preserves_existing_connection_and_endpoint() {
    let db = connect_test_database("optional_reconcile").await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    let catalog = no_auth_catalog();
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let first = opted_in_service(&db, &owner, &catalog.id).await;

    insert_requirement(&db, &catalog.id, false).await;
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let second = opted_in_service(&db, &owner, &catalog.id).await;
    assert_eq!(first.id, second.id);
    assert_eq!(first.endpoint_id, second.endpoint_id);
    assert_endpoint_count(&db, &owner, 1).await;
}

#[tokio::test]
async fn legacy_master_optional_requirement_suspends_explicit_connection_without_removal() {
    let db = connect_test_database("optional_legacy_master")
        .await
        .unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    let mut catalog = no_auth_catalog();
    catalog.auth_method = "bearer".into();
    catalog.auth_key_name = "Authorization".into();
    catalog.service_category = "internal".into();
    catalog.visibility = "public".into();
    catalog.service_type = "http".into();
    catalog.platform_key = None;
    catalog.provider_config_id = None;
    // Provisioning and eligibility inspect metadata without decrypting this value.
    catalog.credential_encrypted = vec![1, 2, 3];
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();

    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let service = opted_in_service(&db, &owner, &catalog.id).await;
    assert_eq!(service.auth_method, "bearer");
    assert_eq!(service.auth_key_name, "Authorization");
    let endpoint = db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(doc! { "_id": &service.endpoint_id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(endpoint.user_id, owner);
    assert_eq!(
        endpoint.catalog_service_id.as_deref(),
        Some(catalog.id.as_str())
    );
    assert_endpoint_count(&db, &owner, 1).await;
    verify_auto_provision_eligibility(&db, &service, &owner)
        .await
        .expect("legacy master without provider requirements remains eligible");

    insert_requirement(&db, &catalog.id, false).await;
    assert!(matches!(
        verify_auto_provision_eligibility(&db, &service, &owner).await,
        Err(AppError::NotFound(_))
    ));
    // Live eligibility rejects use while preserving the explicit connection.
    let held = opted_in_service(&db, &owner, &catalog.id).await;
    assert_eq!(held.id, service.id);
    assert_eq!(held.endpoint_id, endpoint.id);
    assert_endpoint_count(&db, &owner, 1).await;

    // Retrying Connect cannot erase the suspended connection or create an orphan.
    for _ in 0..2 {
        crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
        assert_eq!(
            db.collection::<UserService>(USER_SERVICES)
                .count_documents(doc! { "user_id": &owner, "catalog_service_id": &catalog.id })
                .await
                .unwrap(),
            1
        );
        assert_endpoint_count(&db, &owner, 1).await;
    }
}

#[tokio::test]
async fn optional_runtime_eligibility_rejects_new_required_requirement_immediately() {
    let db = connect_test_database("optional_runtime").await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    let catalog = no_auth_catalog();
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let service = opted_in_service(&db, &owner, &catalog.id).await;
    let requirement = insert_requirement(&db, &catalog.id, false).await;
    verify_auto_provision_eligibility(&db, &service, &owner)
        .await
        .expect("optional provider must not make an automatic route return 404");

    db.collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .update_one(
            doc! { "_id": &requirement },
            doc! { "$set": { "required": true } },
        )
        .await
        .unwrap();
    assert!(matches!(
        verify_auto_provision_eligibility(&db, &service, &owner).await,
        Err(AppError::NotFound(_))
    ));
    // No listing/reconciliation has run since the change.
    assert_endpoint_count(&db, &owner, 1).await;
}

#[tokio::test]
async fn optional_to_required_suspends_and_optional_again_preserves_explicit_identity() {
    let db = connect_test_database("optional_transition").await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    let catalog = no_auth_catalog();
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    insert_requirement(&db, &catalog.id, false).await;
    let requirement = insert_requirement(&db, &catalog.id, false).await;
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let first = opted_in_service(&db, &owner, &catalog.id).await;

    db.collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .update_one(
            doc! { "_id": &requirement },
            doc! { "$set": { "required": true } },
        )
        .await
        .unwrap();
    // A mixed optional/required set suspends use without erasing explicit consent.
    for _ in 0..2 {
        crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
        assert_eq!(
            db.collection::<UserService>(USER_SERVICES)
                .count_documents(doc! { "user_id": &owner })
                .await
                .unwrap(),
            1
        );
        assert_endpoint_count(&db, &owner, 1).await;
    }
    db.collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .update_one(
            doc! { "_id": &requirement },
            doc! { "$set": { "required": false } },
        )
        .await
        .unwrap();
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let replacement = opted_in_service(&db, &owner, &catalog.id).await;
    assert_eq!(first.id, replacement.id);
    assert_eq!(first.endpoint_id, replacement.endpoint_id);
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let repeated = opted_in_service(&db, &owner, &catalog.id).await;
    assert_eq!(replacement.id, repeated.id);
    assert_eq!(replacement.endpoint_id, repeated.endpoint_id);
    assert_endpoint_count(&db, &owner, 1).await;
}

#[tokio::test]
async fn optional_provider_does_not_relax_other_eligibility_guards() {
    let db = connect_test_database("optional_guards").await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    for guard in [
        "private",
        "credential",
        "ssh",
        "category",
        "inactive",
        "auth",
    ] {
        let mut catalog = no_auth_catalog();
        match guard {
            "private" => catalog.visibility = "private".into(),
            "credential" => catalog.requires_user_credential = true,
            "ssh" => catalog.service_type = "ssh".into(),
            "category" => catalog.service_category = "provider".into(),
            "inactive" => catalog.is_active = false,
            "auth" => catalog.auth_method = "bearer".into(),
            _ => unreachable!(),
        }
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();
        insert_requirement(&db, &catalog.id, false).await;
    }
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    assert_eq!(
        db.collection::<UserService>(USER_SERVICES)
            .count_documents(doc! { "user_id": &owner })
            .await
            .unwrap(),
        0
    );
    assert_endpoint_count(&db, &owner, 0).await;
}

#[tokio::test]
async fn optional_requirement_missing_required_flag_fails_closed() {
    let db = connect_test_database("optional_malformed").await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    let catalog = no_auth_catalog();
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.unwrap();
    let service = opted_in_service(&db, &owner, &catalog.id).await;
    let requirement = insert_requirement(&db, &catalog.id, false).await;
    db.collection::<bson::Document>(REQUIREMENTS)
        .update_one(
            doc! { "_id": &requirement },
            doc! { "$unset": { "required": "" } },
        )
        .await
        .unwrap();
    assert!(
        verify_auto_provision_eligibility(&db, &service, &owner)
            .await
            .is_err()
    );
    assert!(crate::test_utils::connect_catalog_fixture_services(&db, &owner).await.is_err());
    // Reconciliation cannot determine eligibility from malformed policy and
    // must retain the existing row rather than destructively guessing.
    let retained = opted_in_service(&db, &owner, &catalog.id).await;
    assert_eq!(retained.id, service.id);
    assert_eq!(retained.endpoint_id, service.endpoint_id);
}
