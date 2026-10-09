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
    let mut winning_specificity = None;
    let mut published = false;
    for endpoint in endpoints.iter().filter(|endpoint| {
        proxy_authorization::rule_matches(
            &ProxyOperationRule {
                method: endpoint.method.clone(),
                path_template: endpoint.path.clone(),
                ..Default::default()
            },
            method,
            path,
        )
    }) {
        let specificity = endpoint
            .path
            .split('/')
            .filter(|segment| !segment.contains('{'))
            .count();
        let callable = endpoint.publication == PublicationState::Published && endpoint.is_active;
        match winning_specificity {
            None => {
                winning_specificity = Some(specificity);
                published = callable;
            }
            Some(winner) if specificity > winner => {
                winning_specificity = Some(specificity);
                published = callable;
            }
            Some(winner) if specificity == winner => published &= callable,
            _ => {}
        }
    }
    if published {
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

pub async fn gate_unconfigured_public_tool(
    db: &Database,
    service_id: &str,
    method: &str,
    path: &str,
) -> AppResult<()> {
    let Some(service) = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":service_id,"offering_kind":"tool","is_active":true})
        .await?
    else {
        return Ok(());
    };
    if service.credential_encrypted.is_empty()
        && service.service_category == "internal"
        && service.visibility == "public"
        && service.provider_config_id.is_none()
        && service.platform_key.as_ref().is_some_and(|config| {
            config.enabled
                && config.audience == crate::models::downstream_service::PlatformKeyAudience::Public
        })
    {
        let canonical = CanonicalPath::from_rest_decoded(path)?;
        gate(db, &service, method, &canonical).await?;
    }
    Ok(())
}

pub async fn change_publication(
    db: &Database,
    service_id: &str,
    endpoint_ids: &[String],
    state: PublicationState,
    actor: &AuditActor,
    audit_key: &[u8],
) -> AppResult<Vec<ServiceEndpoint>> {
    if endpoint_ids.is_empty() || endpoint_ids.len() > 200 {
        return Err(AppError::ValidationError(
            "Select between 1 and 200 operations".into(),
        ));
    }
    let mut session = db.client().start_session().await?;
    let transaction_db = db.clone();
    let service_id = service_id.to_owned();
    let endpoint_ids = endpoint_ids.to_vec();
    let actor = actor.clone();
    let audit_key = zeroize::Zeroizing::new(audit_key.to_vec());
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = change_publication_in_session(
                &transaction_db,
                session,
                &service_id,
                &endpoint_ids,
                state,
                &actor,
                &audit_key,
            )
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

async fn change_publication_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    service_id: &str,
    endpoint_ids: &[String],
    state: PublicationState,
    actor: &AuditActor,
    audit_key: &[u8],
) -> AppResult<Vec<ServiceEndpoint>> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
    let endpoints: Vec<ServiceEndpoint> = coll
        .find(doc! {"service_id": service_id, "_id": {"$in": endpoint_ids}})
        .sort(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .stream(&mut *session)
        .try_collect()
        .await?;
    if endpoints.len() != endpoint_ids.len() {
        return Err(AppError::NotFound("Endpoint not found".into()));
    }
    let mut result = Vec::new();
    for endpoint in endpoints {
        if endpoint.operation_generation <= 0 || endpoint.operation_generation == i64::MAX {
            return Err(AppError::Conflict(
                "Endpoint has an invalid operation_generation".into(),
            ));
        }
        let state_bson = bson::to_bson(&state).map_err(|e| AppError::Internal(e.to_string()))?;
        let updated = coll.find_one_and_update(doc! {"_id": &endpoint.id, "service_id": service_id, "$or": [{"operation_generation": endpoint.operation_generation}, {"operation_generation": {"$exists": false}}]}, vec![doc! {"$set": {
            "publication": {"$literal": state_bson}, "is_active": state == PublicationState::Published,
            "operation_generation": {"$add": [{"$ifNull": ["$operation_generation", 1_i64]}, 1_i64]},
            "updated_at": bson::DateTime::from_chrono(chrono::Utc::now())
        }}]).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?.ok_or_else(|| AppError::Conflict("Endpoint changed during publication".into()))?;
        audit_service::log_actor_event_in_session(db, session, audit_key, actor, "catalog_endpoint_publication_changed", serde_json::json!({"service_id":service_id,"endpoint_id":endpoint.id,"from":endpoint.publication,"to":state,"actor":actor.user_id})).await?;
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
            async_operation: None,

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

    #[test]
    fn equally_specific_unpublished_rows_fail_closed_in_both_orders() {
        let path = CanonicalPath::from_rest_decoded("/items/42").unwrap();
        for state in [
            PublicationState::Draft,
            PublicationState::Paused,
            PublicationState::Published,
        ] {
            let mut blocked = endpoint("blocked", "/items/{id}", state);
            blocked.is_active = false;
            let published = endpoint("published", "/items/{id}", PublicationState::Published);
            for rows in [
                [published.clone(), blocked.clone()],
                [blocked.clone(), published.clone()],
            ] {
                assert!(matches!(
                    require_published_operation(&rows, "GET", &path),
                    Err(AppError::ToolOperationNotPublished)
                ));
            }
        }
    }

    #[tokio::test]
    async fn bulk_publication_failure_rolls_back_rows_generations_and_audit() {
        let Some(db) = crate::test_utils::connect_test_database("tool_publication_atomic").await
        else {
            return;
        };
        let mut rows = [
            endpoint("first", "/first", PublicationState::Draft),
            endpoint("second", "/second", PublicationState::Draft),
        ];
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows[1].operation_generation = i64::MAX;
        db.collection::<ServiceEndpoint>(COLLECTION_NAME)
            .insert_many(&rows)
            .await
            .unwrap();
        let actor = AuditActor {
            user_id: uuid::Uuid::new_v4().to_string(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        let ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
        assert!(matches!(
            change_publication(
                &db,
                "tool",
                &ids,
                PublicationState::Published,
                &actor,
                &[7; 32]
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        let first = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find_one(doc! {"_id": &rows[0].id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.publication, PublicationState::Draft);
        assert!(!first.is_active);
        assert_eq!(first.operation_generation, 1);
        assert_eq!(
            db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
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
            let changed = change_publication(
                &db,
                &row.service_id,
                &[row.id.clone()],
                state,
                &actor,
                &[7; 32],
            )
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
