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

use futures::TryStreamExt;
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

/// Auto-run endpoint discovery for catalog services that have an
/// `openapi_spec_url` but never had `discover-endpoints` run.
///
/// Historically an admin could set a spec URL on a service and still get no
/// `ServiceEndpoint` rows (and therefore no MCP tools and no workflow
/// operations) until someone manually called
/// `POST /services/{id}/discover-endpoints`. This sweep closes that gap for
/// every active HTTP catalog service with a spec URL and **zero** endpoint
/// rows. Services with any existing rows (active or soft-deleted) are left
/// alone -- discovery already ran there, and re-running it automatically
/// could resurrect rows an admin deliberately removed.
///
/// Spec fetches go through the hardened `fetch_spec_json` path (SSRF
/// checks, size limit, cache), so spec URLs on private/internal hosts are
/// not fetched automatically -- the manual admin discover-endpoints route
/// remains the path for those. Failures are logged per service and never
/// abort the sweep.
pub async fn sync_spec_backed_service_endpoints(db: &mongodb::Database) -> AppResult<()> {
    let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    let services: Vec<DownstreamService> = service_col
        .find(doc! {
            "is_active": true,
            "service_type": "http",
            "openapi_spec_url": { "$type": "string", "$ne": "" },
        })
        .await?
        .try_collect()
        .await?;

    for service in services {
        sync_service_endpoints_from_spec_url(db, &service).await;
    }
    Ok(())
}

/// Fire-and-forget wrapper for the admin create/update handlers: re-reads
/// the service and runs the zero-row-guarded discovery in the background so
/// a slow or unreachable spec URL never delays the HTTP response.
pub fn spawn_spec_endpoint_sync(db: mongodb::Database, service_id: String) {
    tokio::spawn(async move {
        let service = match db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": &service_id, "is_active": true, "service_type": "http" })
            .await
        {
            Ok(Some(service)) => service,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(service_id = %service_id, %error, "Spec endpoint sync: failed to load service");
                return;
            }
        };
        if service
            .openapi_spec_url
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return;
        }
        sync_service_endpoints_from_spec_url(&db, &service).await;
    });
}

/// Zero-row-guarded discovery for one service. Logs and returns on any
/// failure instead of erroring so callers (startup sweep, admin handlers)
/// are never blocked by a broken spec URL.
async fn sync_service_endpoints_from_spec_url(db: &mongodb::Database, service: &DownstreamService) {
    let Some(spec_url) = service.openapi_spec_url.as_deref() else {
        return;
    };

    let existing_rows = match db
        .collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .count_documents(doc! { "service_id": &service.id })
        .await
    {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(slug = %service.slug, %error, "Spec endpoint sync: failed to count endpoints");
            return;
        }
    };
    if existing_rows > 0 {
        return; // Discovery already ran (or rows were curated); leave alone.
    }

    let spec = match crate::services::api_docs_service::fetch_spec_json(spec_url).await {
        Ok(spec) => spec,
        Err(error) => {
            tracing::warn!(
                slug = %service.slug,
                spec_url = %crate::services::api_docs_service::redact_url_for_logs(spec_url),
                %error,
                "Spec endpoint sync: failed to fetch OpenAPI spec; run discover-endpoints manually"
            );
            return;
        }
    };

    let inputs = match endpoint_inputs_from_spec(&spec) {
        Ok(inputs) if !inputs.is_empty() => inputs,
        Ok(_) => {
            tracing::warn!(slug = %service.slug, "Spec endpoint sync: spec contained no operations");
            return;
        }
        Err(error) => {
            tracing::warn!(slug = %service.slug, %error, "Spec endpoint sync: spec failed validation");
            return;
        }
    };

    let count = inputs.len();
    match upsert_endpoints_additive(db, &service.id, inputs).await {
        Ok(_) => {
            tracing::info!(
                slug = %service.slug,
                service_id = %service.id,
                endpoint_count = count,
                "Auto-discovered service endpoints from openapi_spec_url"
            );
        }
        Err(error) => {
            tracing::warn!(slug = %service.slug, %error, "Spec endpoint sync: failed to upsert endpoints");
        }
    }
}

/// Parse and validate the hosted overlay for a slug into endpoint inputs.
fn seeded_endpoint_inputs(slug: &str) -> AppResult<Vec<EndpointInput>> {
    let spec = catalog_spec_registry::spec_for_slug(slug).ok_or_else(|| {
        crate::errors::AppError::Internal(format!("no hosted catalog spec registered for '{slug}'"))
    })?;
    endpoint_inputs_from_spec(&spec)
}

/// Parse and validate an OpenAPI document into endpoint inputs, applying
/// the same per-endpoint validation as the admin discover-endpoints route.
fn endpoint_inputs_from_spec(spec: &serde_json::Value) -> AppResult<Vec<EndpointInput>> {
    let parsed = openapi_parser::parse_openapi_spec_value(spec)?;
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
            risk: endpoint.risk,
            supports_idempotency_key: endpoint.supports_idempotency_key,
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
    async fn spec_backed_sweep_discovers_admin_service_endpoints_once() {
        let Some(db) = crate::test_utils::connect_test_database("catalog_spec_url_sweep").await
        else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let _cache_guard = crate::services::api_docs_service::SpecCacheTestGuard::acquire();

        const SPEC_URL: &str = "https://example.com/admin-service-openapi.json";
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = "admin-custom-api".to_string();
        service.created_by = uuid::Uuid::new_v4().to_string();
        service.openapi_spec_url = Some(SPEC_URL.to_string());
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert admin service");

        crate::services::api_docs_service::cache_test_spec(
            SPEC_URL,
            None,
            serde_json::json!({
                "openapi": "3.1.0",
                "info": { "title": "Admin API", "version": "1.0.0" },
                "paths": {
                    "/widgets": {
                        "get": {
                            "operationId": "list_widgets",
                            "responses": {
                                "200": {
                                    "description": "ok",
                                    "content": {
                                        "application/json": {
                                            "schema": { "type": "object" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }),
        );

        sync_spec_backed_service_endpoints(&db)
            .await
            .expect("spec-backed sweep");

        let endpoints = crate::services::service_endpoint_service::list_endpoints(&db, &service.id)
            .await
            .expect("list endpoints");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].name, "list_widgets");
        assert_eq!(endpoints[0].method, "GET");
        assert_eq!(endpoints[0].path, "/widgets");

        // A service that already has rows must be left alone: replace the
        // cached spec with a different document and re-run the sweep.
        crate::services::api_docs_service::cache_test_spec(
            SPEC_URL,
            None,
            serde_json::json!({
                "openapi": "3.1.0",
                "info": { "title": "Admin API", "version": "2.0.0" },
                "paths": {
                    "/gadgets": {
                        "get": {
                            "operationId": "list_gadgets",
                            "responses": {
                                "200": { "description": "ok" }
                            }
                        }
                    }
                }
            }),
        );
        sync_spec_backed_service_endpoints(&db)
            .await
            .expect("second sweep");
        let after = crate::services::service_endpoint_service::list_endpoints(&db, &service.id)
            .await
            .expect("list endpoints again");
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].name, "list_widgets",
            "existing rows must not be replaced"
        );
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
