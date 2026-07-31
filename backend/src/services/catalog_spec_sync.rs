//! Startup sync of hosted catalog overlay specs into `ServiceEndpoint` rows.
//!
//! `/api/v1/mcp/config` publishes operations for catalog-backed user
//! services exclusively from pre-parsed `ServiceEndpoint` rows, and
//! consumers like Aevatar workflow admission fail closed on services with
//! an empty operation set (issue #1290). Historically those rows only
//! existed after an admin manually called
//! `POST /services/{id}/discover-endpoints`, so seeded services published
//! nothing. This sync parses every embedded overlay from
//! `catalog_spec_registry` at startup and additively upserts the resulting
//! endpoints, making the in-tree overlay the source of truth for
//! seeded-endpoint definitions:
//!
//! - endpoints named by the overlay are created or updated to match it;
//! - endpoints an admin added under other names are never touched or
//!   soft-deleted (unlike the admin discover-endpoints route, which
//!   reconciles the full set).

use mongodb::bson::doc;

use crate::errors::AppResult;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::services::service_endpoint_service::{
    EndpointInput, upsert_endpoints_additive, validate_request_content_type,
    validate_response_contract,
};
use crate::services::{catalog_spec_registry, openapi_parser};

/// Materialize `ServiceEndpoint` rows for every seeded catalog service
/// that has a hosted overlay spec. Idempotent; called at startup after
/// `seed_default_services`.
pub async fn sync_seeded_service_endpoints(db: &mongodb::Database) -> AppResult<()> {
    let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);

    for slug in catalog_spec_registry::hydrated_slugs() {
        let Some(service) = service_col
            .find_one(doc! { "slug": slug, "created_by": "system" })
            .await?
        else {
            continue; // Service not seeded on this deployment
        };

        let inputs = match seeded_endpoint_inputs(slug) {
            Ok(inputs) => inputs,
            Err(error) => {
                // Embedded specs are validated by unit tests; reaching this
                // arm means a broken overlay shipped. Keep the deployment
                // booting and surface the defect loudly.
                tracing::error!(slug, %error, "Embedded catalog spec failed to parse; skipping endpoint sync");
                continue;
            }
        };

        let count = inputs.len();
        upsert_endpoints_additive(db, &service.id, inputs).await?;
        tracing::debug!(
            slug,
            service_id = %service.id,
            endpoint_count = count,
            "Synced seeded catalog spec endpoints"
        );
    }

    Ok(())
}

/// Parse and validate the hosted overlay for a slug into endpoint inputs.
fn seeded_endpoint_inputs(slug: &str) -> AppResult<Vec<EndpointInput>> {
    let spec = catalog_spec_registry::spec_for_slug(slug).ok_or_else(|| {
        crate::errors::AppError::Internal(format!("no hosted catalog spec registered for '{slug}'"))
    })?;

    let parsed = openapi_parser::parse_openapi_spec_value(&spec)?;
    let mut inputs = Vec::with_capacity(parsed.len());
    for endpoint in parsed {
        if let Some(content_type) = endpoint.request_content_type.as_deref() {
            validate_request_content_type(content_type)?;
        }
        validate_response_contract(&endpoint.response)?;

        inputs.push(EndpointInput {
            name: endpoint.name,
            description: endpoint.description,
            method: endpoint.method,
            path: endpoint.path,
            parameters: endpoint.parameters,
            request_body_schema: endpoint.request_body_schema,
            request_content_type: endpoint.request_content_type,
            request_body_required: endpoint.request_body_required,
            response_description: None,
            response: endpoint.response,
        });
    }
    Ok(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hydrated_slug_produces_valid_endpoint_inputs() {
        for slug in catalog_spec_registry::hydrated_slugs() {
            let inputs = seeded_endpoint_inputs(slug)
                .unwrap_or_else(|error| panic!("slug '{slug}' failed: {error:?}"));
            assert!(!inputs.is_empty(), "slug '{slug}' produced no endpoints");
            for input in &inputs {
                assert!(
                    input.path.starts_with('/'),
                    "slug '{slug}' endpoint '{}' path must start with '/'",
                    input.name
                );
                assert!(
                    matches!(
                        input.method.as_str(),
                        "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
                    ),
                    "slug '{slug}' endpoint '{}' has unsupported method {}",
                    input.name,
                    input.method
                );
            }
        }
    }

    #[tokio::test]
    async fn sync_populates_endpoints_for_seeded_services() {
        let Some(db) = crate::test_utils::connect_test_database("catalog_spec_sync").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = crate::test_utils::test_encryption_keys();
        crate::services::provider_service::seed_default_providers(&db, &enc)
            .await
            .expect("seed providers");
        crate::services::provider_service::seed_default_services(&db, &enc)
            .await
            .expect("seed services");

        sync_seeded_service_endpoints(&db)
            .await
            .expect("sync endpoints");

        let service = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "slug": "api-lark-bot" })
            .await
            .expect("query service")
            .expect("api-lark-bot seeded");

        let endpoints = crate::services::service_endpoint_service::list_endpoints(&db, &service.id)
            .await
            .expect("list endpoints");
        assert!(
            endpoints
                .iter()
                .any(|ep| ep.name == "bitable_records_search" && ep.method == "POST"),
            "api-lark-bot should publish bitable_records_search"
        );
        assert!(
            endpoints
                .iter()
                .any(|ep| ep.name == "im_message_create" && ep.path == "/open-apis/im/v1/messages"),
            "api-lark-bot should publish im_message_create"
        );

        // Re-running must stay idempotent (same rows, no duplicates).
        sync_seeded_service_endpoints(&db)
            .await
            .expect("second sync");
        let after = crate::services::service_endpoint_service::list_endpoints(&db, &service.id)
            .await
            .expect("list endpoints again");
        assert_eq!(endpoints.len(), after.len());
    }
}
