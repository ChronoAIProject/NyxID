//! Atomic platform-wide route selection for titles and learning, never authority.
use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        platform_settings::{
            COLLECTION_NAME, PLATFORM_SETTINGS_ID, PlatformSettings, UtilityInference,
        },
    },
};
use mongodb::{
    Database,
    bson::{self, doc},
};

pub async fn load(db: &Database) -> AppResult<Option<UtilityInference>> {
    Ok(super::platform_settings_service::load_settings(db)
        .await?
        .utility_inference)
}

pub async fn seed(db: &Database) -> AppResult<()> {
    let rows = db.collection::<PlatformSettings>(COLLECTION_NAME);
    rows.update_one(
        doc! {"_id": PLATFORM_SETTINGS_ID},
        doc! {"$setOnInsert": {"_id": PLATFORM_SETTINGS_ID}},
    )
    .upsert(true)
    .await?;
    // Separate guarded update: an admin edit/explicit null always wins, including
    // an edit racing startup. Other platform settings are never replaced.
    rows.update_one(doc! {"_id": PLATFORM_SETTINGS_ID, "utility_inference": bson::Bson::Null,
        "utility_inference_admin_modified": {"$ne": true}},
        doc! {"$set": {"utility_inference": {"service_slug": "chrono-llm-public", "model": "gpt-6-luna"}}})
        .await?;
    Ok(())
}

pub async fn set(db: &Database, config: Option<UtilityInference>) -> AppResult<()> {
    if let Some(config) = &config {
        if !super::assistant_oneshot_inference::valid_model_id(&config.model)
            || config.service_slug.is_empty()
            || config.service_slug.len() > 128
        {
            return Err(AppError::ValidationError(
                "Invalid utility inference service or model ID".into(),
            ));
        }
        let service = db.collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"slug": &config.service_slug, "is_active": true, "service_type": "http", "inference.model_list": true})
            .await?.ok_or_else(|| AppError::ValidationError("Utility service must be an active catalog inference service with model discovery".into()))?;
        if service.slug == "llm-nyx" || !super::platform_key_service::has_platform_key(&service) {
            return Err(AppError::ValidationError(
                "Utility inference requires a platform-key text inference service".into(),
            ));
        }
    }
    db.collection::<PlatformSettings>(COLLECTION_NAME).update_one(doc! {"_id": PLATFORM_SETTINGS_ID},
        doc! {"$set": {"utility_inference": bson::to_bson(&config).map_err(|_| AppError::Internal("Utility settings serialization failed".into()))?,
            "utility_inference_admin_modified": true}}).upsert(true).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant_oneshot_inference::tests::utility_fixture;
    use mongodb::bson::Document;

    #[tokio::test]
    async fn utility_inference_seed_override_and_explicit_clear_survive_restart() {
        let (state, service, _) = utility_fixture().await;
        let expected = UtilityInference {
            service_slug: "chrono-llm-public".into(),
            model: "gpt-6-luna".into(),
        };
        assert_eq!(load(&state.db).await.unwrap(), Some(expected));
        let mut alternate = service.clone();
        alternate.id = uuid::Uuid::new_v4().to_string();
        alternate.slug = "custom-utility".into();
        state
            .db
            .collection::<DownstreamService>(SERVICES)
            .insert_one(&alternate)
            .await
            .unwrap();
        let override_config = UtilityInference {
            service_slug: alternate.slug,
            model: "gpt-4.1-mini".into(),
        };
        set(&state.db, Some(override_config.clone())).await.unwrap();
        seed(&state.db).await.unwrap();
        assert_eq!(load(&state.db).await.unwrap(), Some(override_config));
        set(&state.db, None).await.unwrap();
        seed(&state.db).await.unwrap();
        assert_eq!(load(&state.db).await.unwrap(), None);
        // An unrelated settings writer cannot erase the utility clear marker.
        crate::services::platform_settings_service::update_broker_settings(
            &state.db,
            crate::services::platform_settings_service::BrokerSettingsPatch {
                broker_require_sender_constraint: Some(Some(true)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        seed(&state.db).await.unwrap();
        assert_eq!(load(&state.db).await.unwrap(), None);
        let row = state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": PLATFORM_SETTINGS_ID})
            .await
            .unwrap()
            .unwrap();
        assert!(row.get_bool("utility_inference_admin_modified").unwrap());
        assert!(row.get_bool("broker_require_sender_constraint").unwrap());
    }

    #[tokio::test]
    async fn utility_inference_validates_catalog_and_model() {
        let (state, service, _) = utility_fixture().await;
        for (slug, model) in [
            ("missing", "gpt-6-luna"),
            (service.slug.as_str(), ""),
            (service.slug.as_str(), "model\nunsafe"),
            (service.slug.as_str(), &"x".repeat(257)),
        ] {
            assert!(
                set(
                    &state.db,
                    Some(UtilityInference {
                        service_slug: slug.into(),
                        model: model.into()
                    })
                )
                .await
                .is_err()
            );
        }
        state
            .db
            .collection::<Document>(SERVICES)
            .update_one(doc! {"_id":service.id}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        assert!(
            set(
                &state.db,
                Some(UtilityInference {
                    service_slug: service.slug,
                    model: "gpt-6-luna".into()
                })
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn utility_inference_absent_config_preserves_legacy_selection() {
        let db =
            crate::test_utils::connect_transaction_test_database("utility_inference_legacy").await;
        assert_eq!(load(&db).await.unwrap(), None);
        db.collection::<Document>(COLLECTION_NAME)
            .insert_one(doc! {"_id": PLATFORM_SETTINGS_ID, "broker_policy_revision":2})
            .await
            .unwrap();
        assert_eq!(load(&db).await.unwrap(), None);
    }
}
