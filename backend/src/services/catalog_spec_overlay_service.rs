use crate::{
    errors::{AppError, AppResult},
    models::{
        catalog_spec_overlay::{COLLECTION_NAME, CatalogSpecOverlay},
        downstream_service::{CatalogImportSource, DownstreamService},
    },
};
use mongodb::{
    Database,
    bson::{self, doc},
};
use sha2::{Digest, Sha256};

pub fn validate_document(document: &serde_json::Value) -> AppResult<String> {
    if !document.is_object()
        || !document["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3."))
        || !document["paths"].is_object()
    {
        return Err(AppError::ValidationError(
            "Overlay requires an OpenAPI 3.x object with paths".into(),
        ));
    }
    let bytes =
        serde_json::to_vec(document).map_err(|e| AppError::ValidationError(e.to_string()))?;
    if bytes.len() > 1024 * 1024 {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes: 1024 * 1024,
            context: "catalog spec overlay".into(),
        });
    }
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub async fn get(db: &Database, service_id: &str) -> AppResult<Option<CatalogSpecOverlay>> {
    Ok(db
        .collection::<CatalogSpecOverlay>(COLLECTION_NAME)
        .find_one(doc! {"service_id": service_id})
        .await?)
}

#[derive(serde::Serialize)]
pub struct SyncCounts {
    pub operations_synced: usize,
    pub operations_added: usize,
    pub operations_changed: usize,
}

pub async fn put(
    db: &Database,
    service: &DownstreamService,
    document: serde_json::Value,
    mut source: Option<CatalogImportSource>,
    actor_id: &str,
    base_url: &str,
) -> AppResult<(CatalogSpecOverlay, SyncCounts)> {
    let sha256 = validate_document(&document)?;
    let inputs = super::catalog_spec_sync::destination_endpoint_inputs(service, &document)?;
    if let Some(source) = source.as_mut() {
        source.imported_at = Some(chrono::Utc::now());
    }
    let mut proposed = service.clone();
    proposed.import_source = source.clone();
    super::tool_topics::validate_tool_service(&proposed)?;
    let coll = db.collection::<CatalogSpecOverlay>(COLLECTION_NAME);
    let now = chrono::Utc::now();
    let previous = get(db, &service.id).await?;
    let overlay = CatalogSpecOverlay {
        id: previous
            .as_ref()
            .map(|o| o.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        service_id: service.id.clone(),
        previous_document: previous.as_ref().map(|o| o.document.clone()),
        document,
        sha256,
        revision: previous.as_ref().map(|o| o.revision + 1).unwrap_or(1),
        source,
        created_by: previous
            .as_ref()
            .map(|o| o.created_by.clone())
            .unwrap_or_else(|| actor_id.into()),
        created_at: previous.as_ref().map(|o| o.created_at).unwrap_or(now),
        updated_at: now,
    };
    if let Some(previous) = previous {
        let result = coll
            .replace_one(
                doc! {"_id": &previous.id, "revision": previous.revision},
                &overlay,
            )
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::Conflict("Overlay changed during import".into()));
        }
    } else {
        coll.insert_one(&overlay).await?;
    }
    let existing = super::service_endpoint_service::list_all_endpoints(db, &service.id).await?;
    let count = SyncCounts {
        operations_synced: inputs.len(),
        operations_added: inputs
            .iter()
            .filter(|input| !existing.iter().any(|row| row.name == input.name))
            .count(),
        operations_changed: inputs
            .iter()
            .filter(|input| {
                existing.iter().any(|row| {
                    row.name == input.name
                        && (row.method != input.method
                            || row.path != input.path
                            || row.description != input.description
                            || row.parameters != input.parameters
                            || row.request_body_schema != input.request_body_schema
                            || row.response_description != input.response_description)
                })
            })
            .count(),
    };
    super::service_endpoint_service::upsert_endpoints_additive(db, &service.id, inputs).await?;
    let mut set = doc! {"import_source": bson::to_bson(&overlay.source).map_err(|e| AppError::Internal(e.to_string()))?, "updated_at": bson::DateTime::from_chrono(now)};
    if service.openapi_spec_url.is_none() {
        set.insert(
            "openapi_spec_url",
            format!(
                "{}/api/v1/catalog-specs/service/{}/openapi.json",
                base_url.trim_end_matches('/'),
                service.id
            ),
        );
    }
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(doc! {"_id": &service.id}, doc! {"$set": set})
        .await?;
    Ok((overlay, count))
}

pub async fn delete(db: &Database, service_id: &str) -> AppResult<()> {
    db.collection::<CatalogSpecOverlay>(COLLECTION_NAME)
        .delete_one(doc! {"service_id":service_id})
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_validation_bounds_and_fingerprints_document() {
        let spec = serde_json::json!({"openapi":"3.1.0","paths":{}});
        assert_eq!(validate_document(&spec).unwrap().len(), 64);
        assert_eq!(
            validate_document(&spec).unwrap(),
            validate_document(&spec).unwrap()
        );
        for invalid in [
            serde_json::json!([]),
            serde_json::json!({"openapi":"2.0","paths":{}}),
            serde_json::json!({"openapi":"3.0.0","paths":[]}),
        ] {
            assert!(validate_document(&invalid).is_err());
        }
        let oversized =
            serde_json::json!({"openapi":"3.1.0","paths":{},"description":"x".repeat(1024 * 1024)});
        assert!(matches!(
            validate_document(&oversized),
            Err(AppError::RequestBodyTooLarge { .. })
        ));
    }

    #[tokio::test]
    async fn replacement_retains_previous_document_and_deletion_keeps_operations() {
        let Some(db) = crate::test_utils::connect_test_database("tool_overlay_history").await
        else {
            return;
        };
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.offering_kind = crate::models::downstream_service::OfferingKind::Tool;
        service.service_category = "internal".into();
        service.auth_method = "none".into();
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();
        let first = serde_json::json!({"openapi":"3.1.0","paths":{"/first":{"get":{"operationId":"first","responses":{"200":{"description":"ok"}}}}}});
        let second = serde_json::json!({"openapi":"3.1.0","paths":{"/second":{"get":{"operationId":"second","responses":{"200":{"description":"ok"}}}}}});
        let (row, _) = put(
            &db,
            &service,
            first.clone(),
            None,
            "admin",
            "https://nyxid.test",
        )
        .await
        .unwrap();
        assert_eq!(row.revision, 1);
        let (row, _) = put(&db, &service, second, None, "admin", "https://nyxid.test")
            .await
            .unwrap();
        assert_eq!(row.revision, 2);
        assert_eq!(row.previous_document, Some(first));
        assert_eq!(
            super::super::service_endpoint_service::list_all_endpoints(&db, &service.id)
                .await
                .unwrap()
                .len(),
            2
        );
        delete(&db, &service.id).await.unwrap();
        assert!(get(&db, &service.id).await.unwrap().is_none());
        assert_eq!(
            super::super::service_endpoint_service::list_all_endpoints(&db, &service.id)
                .await
                .unwrap()
                .len(),
            2
        );
        db.drop().await.unwrap();
    }
}
