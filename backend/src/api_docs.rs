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
        crate::handlers::proxy::proxy_request,
        crate::handlers::proxy::proxy_request_by_slug,
        crate::handlers::services::list_services,
        crate::handlers::services::create_service,
        crate::handlers::services::get_service,
        crate::handlers::services::update_service,
        crate::handlers::services::delete_service,
        crate::handlers::services::get_oidc_credentials,
        crate::handlers::services::update_redirect_uris,
        crate::handlers::services::regenerate_oidc_secret,
        crate::handlers::ssh_tunnel::issue_ssh_certificate,
        crate::handlers::ssh_tunnel::ssh_tunnel_ws,
        // AI Services (unified key management)
        crate::handlers::keys::create_key,
        crate::handlers::keys::list_keys,
        crate::handlers::keys::get_key,
        crate::handlers::keys::update_key,
        crate::handlers::keys::delete_key,
        // Catalog
        crate::handlers::catalog::list_catalog,
        crate::handlers::catalog::get_catalog_entry,
        crate::handlers::catalog::list_catalog_endpoints,
        // Endpoints
        crate::handlers::user_endpoints::list_endpoints,
        crate::handlers::user_endpoints::update_endpoint,
        crate::handlers::user_endpoints::delete_endpoint,
        crate::handlers::user_endpoints::list_openapi_endpoints,
        // External API Keys
        crate::handlers::user_api_keys_external::list_external_api_keys,
        crate::handlers::user_api_keys_external::update_external_api_key,
        crate::handlers::user_api_keys_external::delete_external_api_key,
        // User Services
        crate::handlers::user_services_handler::list_user_services,
        crate::handlers::user_services_handler::update_user_service,
        crate::handlers::user_services_handler::delete_user_service,
        // NyxID API Keys
        crate::handlers::api_keys::list_keys,
        crate::handlers::api_keys::get_key,
        crate::handlers::api_keys::create_key,
        crate::handlers::api_keys::update_key,
        crate::handlers::api_keys::delete_key,
        crate::handlers::api_keys::rotate_key
    ),
    components(
        schemas(
            crate::errors::ErrorResponse,
            crate::handlers::services::CreateServiceRequest,
            crate::handlers::services::SshServiceConfigRequest,
            crate::handlers::services::SshServiceConfigResponse,
            crate::handlers::services::UpdateServiceRequest,
            crate::handlers::services::ServiceResponse,
            crate::handlers::services::ServiceListResponse,
            crate::handlers::services_helpers::DeleteServiceResponse,
            crate::handlers::services::OidcCredentialsResponse,
            crate::handlers::services::UpdateRedirectUrisRequest,
            crate::handlers::services::RedirectUrisResponse,
            crate::handlers::services::RegenerateSecretResponse,
            crate::handlers::proxy::ProxyServiceItem,
            crate::handlers::proxy::ProxyServicesResponse,
            crate::handlers::ssh_tunnel::IssueSshCertificateRequest,
            crate::handlers::ssh_tunnel::IssueSshCertificateResponse,
            // AI Services
            crate::handlers::keys::CreateKeyRequest,
            crate::handlers::keys::UpdateKeyRequest,
            crate::handlers::keys::KeyResponse,
            crate::handlers::keys::KeyListResponse,
            crate::handlers::keys::DeleteKeyResponse,
            // Catalog
            crate::handlers::catalog::CatalogEntryResponse,
            crate::handlers::catalog::CatalogListResponse,
            crate::handlers::catalog::CatalogEndpointResponse,
            crate::handlers::catalog::CatalogEndpointsListResponse,
            crate::models::downstream_service::ServiceCapabilities,
            // Endpoints
            crate::handlers::user_endpoints::UpdateEndpointRequest,
            crate::handlers::user_endpoints::EndpointResponse,
            crate::handlers::user_endpoints::EndpointListResponse,
            crate::handlers::user_endpoints::UserEndpointOperationResponse,
            crate::handlers::user_endpoints::UserEndpointOperationsResponse,
            // External API Keys
            crate::handlers::user_api_keys_external::UpdateExternalApiKeyRequest,
            crate::handlers::user_api_keys_external::ExternalApiKeyResponse,
            crate::handlers::user_api_keys_external::ExternalApiKeyListResponse,
            // User Services
            crate::handlers::user_services_handler::UpdateUserServiceRequest,
            crate::handlers::user_services_handler::UserServiceResponse,
            crate::handlers::user_services_handler::UserServiceListResponse,
            // NyxID API Keys
            crate::handlers::api_keys::CreateApiKeyRequest,
            crate::handlers::api_keys::UpdateApiKeyRequest,
            crate::handlers::api_keys::CreateApiKeyResponse,
            crate::handlers::api_keys::AllowedServiceInfo,
            crate::handlers::api_keys::AllowedNodeInfo,
            crate::handlers::api_keys::ApiKeyResponse,
            crate::handlers::api_keys::ApiKeyListResponse,
            crate::handlers::api_keys::DeleteApiKeyResponse
        )
    ),
    tags(
        (name = "Docs", description = "NyxID API documentation endpoints"),
        (name = "Proxy Docs", description = "Downstream OpenAPI and AsyncAPI catalog endpoints"),
        (name = "Services", description = "Downstream service management (admin)"),
        (name = "Proxy", description = "Authenticated downstream service discovery"),
        (name = "SSH", description = "SSH certificate issuance and WebSocket tunnel endpoints"),
        (name = "AI Services", description = "Unified key management: auto-provisions endpoint, credential, and proxy routing from catalog or custom input"),
        (name = "Catalog", description = "Read-only service catalog for users (admin-created services and providers)"),
        (name = "Endpoints", description = "User-managed target URLs"),
        (name = "External API Keys", description = "User's external API keys and credentials"),
        (name = "User Services", description = "User's proxy routing configuration"),
        (name = "API Keys", description = "NyxID API keys with service and node scope")
    )
)]
pub struct ApiDoc;
