#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        crate::handlers::docs::docs_ui,
        crate::handlers::docs::catalog_ui,
        crate::handlers::docs::openapi_json,
        crate::handlers::docs::asyncapi_json,
        crate::handlers::docs::service_docs_ui,
        crate::handlers::docs::service_openapi_json,
        crate::handlers::docs::service_asyncapi_json,
        crate::handlers::proxy::list_proxy_services,
        crate::handlers::services::list_services,
        crate::handlers::services::create_service,
        crate::handlers::services::get_service,
        crate::handlers::services::update_service
    ),
    components(
        schemas(
            crate::errors::ErrorResponse,
            crate::handlers::services::CreateServiceRequest,
            crate::handlers::services::UpdateServiceRequest,
            crate::handlers::services::ServiceResponse,
            crate::handlers::services::ServiceListResponse,
            crate::handlers::proxy::ProxyServiceItem,
            crate::handlers::proxy::ProxyServicesResponse
        )
    ),
    tags(
        (name = "Docs", description = "NyxID API documentation endpoints"),
        (name = "Proxy Docs", description = "Downstream OpenAPI and AsyncAPI catalog endpoints"),
        (name = "Services", description = "Downstream service management"),
        (name = "Proxy", description = "Authenticated downstream service discovery")
    )
)]
pub struct ApiDoc;
