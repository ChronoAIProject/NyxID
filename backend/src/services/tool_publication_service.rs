use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{DownstreamService, OfferingKind, ProxyOperationRule},
        service_endpoint::{COLLECTION_NAME, PublicationState, ServiceEndpoint},
    },
    services::{
        audit_service::{self, AuditActor},
        proxy_authorization::{self, CanonicalPath},
    },
};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Document, doc},
};

pub fn require_published_operation(
    endpoints: &[ServiceEndpoint],
    method: &str,
    path: &CanonicalPath,
) -> AppResult<()> {
    let matched = endpoints
        .iter()
        .filter(|endpoint| {
            proxy_authorization::rule_matches(
                &ProxyOperationRule {
                    method: endpoint.method.clone(),
                    path_template: endpoint.path.clone(),
                },
                method,
                path,
            )
        })
        .max_by_key(|endpoint| {
            endpoint
                .path
                .split('/')
                .filter(|segment| !segment.contains('{'))
                .count()
        });
    if matched.is_some_and(|endpoint| {
        endpoint.publication == PublicationState::Published && endpoint.is_active
    }) {
        Ok(())
    } else {
        Err(AppError::ToolOperationNotPublished)
    }
}

pub async fn gate(
    db: &Database,
    service: &DownstreamService,
    method: &str,
    path: &CanonicalPath,
) -> AppResult<()> {
    if service.offering_kind != OfferingKind::Tool {
        return Ok(());
    }
    let endpoints: Vec<ServiceEndpoint> = db
        .collection::<ServiceEndpoint>(COLLECTION_NAME)
        .find(doc! {"service_id": &service.id})
        .await?
        .try_collect()
        .await?;
    require_published_operation(&endpoints, method, path)
}

pub async fn change_publication(
    db: &Database,
    service_id: &str,
    endpoint_ids: &[String],
    state: PublicationState,
    actor: &AuditActor,
) -> AppResult<Vec<ServiceEndpoint>> {
    if endpoint_ids.is_empty() || endpoint_ids.len() > 200 {
        return Err(AppError::ValidationError(
            "Select between 1 and 200 operations".into(),
        ));
    }
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
    let endpoints: Vec<ServiceEndpoint> = coll
        .find(doc! {"service_id": service_id, "_id": {"$in": endpoint_ids}})
        .await?
        .try_collect()
        .await?;
    if endpoints.len() != endpoint_ids.len() {
        return Err(AppError::NotFound("Endpoint not found".into()));
    }
    let mut result = Vec::new();
    for endpoint in endpoints {
        let state_bson = bson::to_bson(&state).map_err(|e| AppError::Internal(e.to_string()))?;
        let updated = coll.find_one_and_update(doc! {"_id": &endpoint.id, "service_id": service_id, "$or": [{"operation_generation": endpoint.operation_generation}, {"operation_generation": {"$exists": false}}]}, vec![doc! {"$set": {
            "publication": {"$literal": state_bson}, "is_active": state == PublicationState::Published,
            "operation_generation": {"$add": [{"$ifNull": ["$operation_generation", 1_i64]}, 1_i64]},
            "updated_at": bson::DateTime::from_chrono(chrono::Utc::now())
        }}]).return_document(mongodb::options::ReturnDocument::After).await?.ok_or_else(|| AppError::Conflict("Endpoint changed during publication".into()))?;
        audit_service::log_actor_event(db.clone(), actor, "catalog_endpoint_publication_changed", Some(serde_json::json!({"service_id":service_id,"endpoint_id":endpoint.id,"from":endpoint.publication,"to":state,"actor":actor.user_id}))).await?;
        result.push(updated);
    }
    Ok(result)
}

pub async fn ids_by_name(
    db: &Database,
    service_id: &str,
    names: &[String],
) -> AppResult<Vec<String>> {
    if names.is_empty()
        || names.len() > 200
        || names.iter().collect::<std::collections::HashSet<_>>().len() != names.len()
    {
        return Err(AppError::ValidationError(
            "endpoint_names requires 1–200 distinct names".into(),
        ));
    }
    let rows: Vec<Document> = db
        .collection::<Document>(COLLECTION_NAME)
        .find(doc! {"service_id":service_id,"name":{"$in":names}})
        .await?
        .try_collect()
        .await?;
    if rows.len() != names.len() {
        return Err(AppError::NotFound("Endpoint name not found".into()));
    }
    rows.into_iter()
        .map(|row| {
            row.get_str("_id")
                .map(str::to_string)
                .map_err(|e| AppError::Internal(e.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(name: &str, path: &str, publication: PublicationState) -> ServiceEndpoint {
        ServiceEndpoint {
            id: uuid::Uuid::new_v4().to_string(),
            service_id: "tool".into(),
            name: name.into(),
            description: None,
            method: "GET".into(),
            path: path.into(),
            target_id: None,
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: None,
            response: Default::default(),
            risk: None,
            supports_idempotency_key: false,
            data_scope: None,
            cost_class: None,
            execution: Default::default(),
            publication,
            is_active: publication == PublicationState::Published,
            operation_generation: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn publication_gate_matches_paths_and_rejects_unknown_inactive_and_unpublished() {
        let path = CanonicalPath::from_rest_decoded("/items/42").unwrap();
        let mut row = endpoint("item", "/items/{id}", PublicationState::Published);
        assert!(require_published_operation(&[row.clone()], "GET", &path).is_ok());
        assert!(matches!(
            require_published_operation(&[row.clone()], "POST", &path),
            Err(AppError::ToolOperationNotPublished)
        ));
        assert!(matches!(
            require_published_operation(&[], "GET", &path),
            Err(AppError::ToolOperationNotPublished)
        ));
        row.is_active = false;
        assert!(require_published_operation(&[row.clone()], "GET", &path).is_err());
        row.is_active = true;
        for state in [
            PublicationState::Draft,
            PublicationState::Validated,
            PublicationState::Paused,
        ] {
            row.publication = state;
            assert!(require_published_operation(&[row.clone()], "GET", &path).is_err());
        }
    }

    #[test]
    fn specific_draft_cannot_be_bypassed_by_published_template() {
        let rows = [
            endpoint("any", "/items/{id}", PublicationState::Published),
            endpoint("special", "/items/special", PublicationState::Draft),
        ];
        assert!(
            require_published_operation(
                &rows,
                "GET",
                &CanonicalPath::from_rest_decoded("/items/special").unwrap()
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn transitions_advance_generation_and_audit_without_reactivating_drafts() {
        let Some(db) = crate::test_utils::connect_test_database("tool_publication").await else {
            return;
        };
        let mut row = endpoint("item", "/items/{id}", PublicationState::Draft);
        row.service_id = uuid::Uuid::new_v4().to_string();
        db.collection::<ServiceEndpoint>(COLLECTION_NAME)
            .insert_one(&row)
            .await
            .unwrap();
        let actor = AuditActor {
            user_id: uuid::Uuid::new_v4().to_string(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        for (index, state) in [
            PublicationState::Validated,
            PublicationState::Published,
            PublicationState::Paused,
            PublicationState::Draft,
        ]
        .into_iter()
        .enumerate()
        {
            let changed =
                change_publication(&db, &row.service_id, &[row.id.clone()], state, &actor)
                    .await
                    .unwrap();
            assert_eq!(changed[0].publication, state);
            assert_eq!(changed[0].is_active, state == PublicationState::Published);
            assert_eq!(changed[0].operation_generation, index as i64 + 2);
        }
        assert_eq!(
            db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type":"catalog_endpoint_publication_changed"})
                .await
                .unwrap(),
            4
        );
        db.drop().await.unwrap();
    }
}
