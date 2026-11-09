//! Insert-only catalog publication; each Bot connection still requires its owner.
use crate::{
    errors::{AppError, AppResult},
    models::downstream_service::{COLLECTION_NAME, DownstreamService},
};
use mongodb::{
    Database,
    bson::{self, doc},
};

const TEMPLATE: &str = include_str!("../../../../docs/examples/nyxbot-voice-service.template.json");

/// Startup has already installed the active-slug unique index in `db.rs`.
/// Match inactive rows too: restarts must never undo admin edits or deactivation.
pub async fn seed(db: &Database, base_url: &str) -> AppResult<()> {
    let mut fields: bson::Document = serde_json::from_str(TEMPLATE)
        .map_err(|_| AppError::Internal("Invalid embedded NyxBot Voice catalog template".into()))?;
    let now = bson::DateTime::now();
    fields.extend(doc! {
        "_id": uuid::Uuid::new_v4().to_string(),
        "base_url": format!("{}/api/v1/proxy/nyxbot-voice", base_url.trim_end_matches('/')),
        "auth_key_name": "", "credential_encrypted": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic, bytes: Vec::new()
        },
        // A dedicated owner publication flow, never a generic connectable entry.
        "requires_user_credential": false,
        "identity_propagation_mode": "none", "inject_delegation_token": false,
        "is_active": true, "created_by": "system", "created_at": now, "updated_at": now
    });
    let service: DownstreamService = bson::from_document(fields)
        .map_err(|_| AppError::Internal("Invalid embedded NyxBot Voice catalog service".into()))?;
    let insert = bson::to_document(&service)
        .map_err(|_| AppError::Internal("Unable to encode NyxBot Voice catalog service".into()))?;
    let services = db.collection::<DownstreamService>(COLLECTION_NAME);
    let filter = doc! {"slug": super::access::SERVICE_SLUG};
    match services
        .update_one(filter.clone(), doc! {"$setOnInsert": insert})
        .upsert(true)
        .await
    {
        Ok(result) => {
            if result.upserted_id.is_some() {
                tracing::info!(
                    slug = super::access::SERVICE_SLUG,
                    "Seeded NyxBot Voice catalog service"
                );
            }
        }
        // Two replicas can race to insert; only accept a duplicate if the row
        // now exists. Do not hide unrelated database or index errors.
        Err(error)
            if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000)
                || matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(e) if e.code == 11000) =>
        {
            if services.find_one(filter).await?.is_none() {
                return Err(error.into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::catalog_service;

    async fn database(name: &str) -> Database {
        let db = crate::test_utils::connect_transaction_test_database(name).await;
        // Same partial unique index as production; inactive rows remain visible
        // to the seed even though they are outside this index.
        db.collection::<DownstreamService>(COLLECTION_NAME)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"slug": 1})
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(doc! {"is_active": true})
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        db
    }

    #[tokio::test]
    async fn seed_inserts_once_matches_template_and_rerun_is_noop() {
        let db = database("voice_catalog_seed_once").await;
        seed(&db, "https://nyxid.example/deployment/")
            .await
            .unwrap();
        let rows = db.collection::<bson::Document>(COLLECTION_NAME);
        let first = rows.find_one(doc! {}).await.unwrap().unwrap();
        let service: DownstreamService = bson::from_document(first.clone()).unwrap();
        assert_eq!(
            uuid::Uuid::parse_str(&service.id)
                .unwrap()
                .get_version_num(),
            4
        );
        assert_eq!(
            service.base_url,
            "https://nyxid.example/deployment/api/v1/proxy/nyxbot-voice"
        );
        let actual = serde_json::to_value(&service).unwrap();
        let template: serde_json::Value = serde_json::from_str(TEMPLATE).unwrap();
        for (key, expected) in template.as_object().unwrap() {
            if key == "capabilities" {
                let expected: crate::models::downstream_service::ServiceCapabilities =
                    serde_json::from_value(expected.clone()).unwrap();
                assert_eq!(actual[key], serde_json::to_value(expected).unwrap());
            } else if key != "base_url" {
                assert_eq!(&actual[key], expected, "{key}");
            }
        }
        assert!(!service.requires_user_credential);
        assert!(!service.forward_access_token);
        assert!(!service.inject_delegation_token);
        assert_eq!(service.identity_propagation_mode, "none");
        assert!(service.anonymous_endpoints.is_empty());
        assert!(service.credential_encrypted.is_empty());
        assert!(service.provider_config_id.is_none());
        assert!(service.openapi_spec_url.is_none());
        assert!(service.asyncapi_spec_url.is_none());
        seed(&db, "https://changed.example").await.unwrap();
        assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 1);
        assert_eq!(rows.find_one(doc! {}).await.unwrap().unwrap(), first);
        // Both startup discovery passes must leave this protocol unmaterialized.
        crate::services::catalog_spec_sync::sync_seeded_service_endpoints(&db)
            .await
            .unwrap();
        crate::services::catalog_spec_sync::sync_spec_backed_service_endpoints(&db)
            .await
            .unwrap();
        for collection in ["service_endpoints", "user_endpoints", "user_services"] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "{collection}"
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn seed_preserves_admin_edited_active_and_inactive_rows() {
        let db = database("voice_catalog_seed_admin").await;
        let rows = db.collection::<bson::Document>(COLLECTION_NAME);
        for active in [true, false] {
            rows.delete_many(doc! {}).await.unwrap();
            let mut admin = crate::models::downstream_service::test_helpers::dummy_service();
            admin.id = uuid::Uuid::new_v4().to_string();
            admin.slug = super::super::access::SERVICE_SLUG.into();
            admin.name = "Admin's voice service".into();
            admin.base_url = "https://admin.example/custom".into();
            admin.created_by = "admin".into();
            admin.is_active = active;
            let mut before = bson::to_document(&admin).unwrap();
            before.insert("admin_metadata", "must survive unchanged");
            rows.insert_one(&before).await.unwrap();
            seed(&db, "https://nyxid.example").await.unwrap();
            assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 1);
            assert_eq!(rows.find_one(doc! {}).await.unwrap().unwrap(), before);
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_seed_creates_one_row() {
        let db = database("voice_catalog_seed_concurrent").await;
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let tasks = (0..2).map(|_| {
            let db = db.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                seed(&db, "https://nyxid.example").await
            })
        });
        for result in futures::future::join_all(tasks).await {
            result.unwrap().unwrap();
        }
        assert_eq!(
            db.collection::<DownstreamService>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn seed_keeps_default_catalog_listing_unchanged_but_all_can_discover() {
        let db = database("voice_catalog_seed_listing").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection("users")
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let mut ordinary = crate::models::downstream_service::test_helpers::dummy_service();
        ordinary.id = uuid::Uuid::new_v4().to_string();
        ordinary.requires_user_credential = true;
        ordinary.is_active = true;
        db.collection::<DownstreamService>(COLLECTION_NAME)
            .insert_one(&ordinary)
            .await
            .unwrap();
        let before = catalog_service::list_catalog(&db, &state.encryption_keys, &owner)
            .await
            .unwrap();
        seed(&db, &state.config.base_url).await.unwrap();
        let after = catalog_service::list_catalog(&db, &state.encryption_keys, &owner)
            .await
            .unwrap();
        assert_eq!(
            before.iter().map(|s| &s.slug).collect::<Vec<_>>(),
            after.iter().map(|s| &s.slug).collect::<Vec<_>>()
        );
        assert_eq!(after.len(), 1);
        let all = catalog_service::list_catalog_all(&db, &state.encryption_keys, &owner)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        assert!(
            all.iter()
                .any(|s| s.slug == super::super::access::SERVICE_SLUG)
        );
        db.drop().await.unwrap();
    }
}
