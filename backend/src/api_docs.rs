#[derive(utoipa::OpenApi)]
#[openapi(
    modifiers(&SecurityAddon),
    paths(
        crate::handlers::docs::docs_ui,
        crate::handlers::docs::catalog_ui,
        crate::handlers::docs::openapi_json,
        crate::handlers::docs::asyncapi_json,
        crate::handlers::docs::catalog_spec_json,
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
        // Hosted Connect Links
        crate::handlers::connect_links::create_connect_link,
        crate::handlers::connect_links::get_connect_link,
        crate::handlers::connect_links::cancel_connect_link,
        crate::handlers::connect_links::preview_connect_link,
        crate::handlers::connect_links::cancel_hosted_connect_link,
        crate::handlers::connect_links::complete_connect_link,
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
        // Provider connections
        crate::handlers::user_tokens::disconnect_provider,
        // User Services
        crate::handlers::user_services_handler::list_user_services,
        crate::handlers::user_services_handler::update_user_service,
        crate::handlers::user_services_handler::delete_user_service,
        // NyxID API Keys
        crate::handlers::api_keys::list_keys,
        crate::handlers::api_keys::get_key,
        crate::handlers::api_keys::plan_key_scope,
        crate::handlers::api_keys::create_key,
        crate::handlers::api_keys::update_key,
        crate::handlers::api_keys::delete_key,
        crate::handlers::api_keys::rotate_key,
        crate::handlers::api_keys::list_durable_grants,
        crate::handlers::api_keys::revoke_durable_grant,
        crate::handlers::api_keys::reauthorize_durable_grants,
        // Billing
        crate::handlers::billing::get_usage,
        crate::handlers::billing::get_wallet,
        crate::handlers::billing::provision_wallet,
        crate::handlers::billing::create_topup,
        crate::handlers::billing::list_topups,
        crate::handlers::billing::download_invoice,
        // Demo
        crate::handlers::demo::get_demo
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
            // Hosted Connect Links
            crate::handlers::connect_links::CreateConnectLinkRequest,
            crate::handlers::connect_links::CreateConnectLinkResponse,
            crate::handlers::connect_links::PreviewConnectLinkRequest,
            crate::handlers::connect_links::PreviewConnectLinkResponse,
            crate::handlers::connect_links::ConnectedServiceResponse,
            crate::handlers::connect_links::ConnectLinkStatusResponse,
            crate::handlers::connect_links::CancelHostedConnectLinkRequest,
            crate::handlers::connect_links::CompleteConnectLinkRequest,
            crate::handlers::connect_links::CompleteConnectLinkResponse,
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
            crate::handlers::user_api_keys_external::DeleteExternalApiKeyResponse,
            // Provider connections
            crate::handlers::user_tokens::DisconnectProviderResponse,
            // User Services
            crate::handlers::user_services_handler::UpdateUserServiceRequest,
            crate::handlers::user_services_handler::UserServiceResponse,
            crate::handlers::user_services_handler::UserServiceListResponse,
            // NyxID API Keys
            crate::handlers::api_keys::CreateApiKeyRequest,
            crate::handlers::api_keys::UpdateApiKeyRequest,
            crate::handlers::api_keys::ApiKeyScopePlanRequest,
            crate::handlers::api_keys::CreateApiKeyResponse,
            crate::handlers::api_keys::AllowedServiceInfo,
            crate::handlers::api_keys::AllowedNodeInfo,
            crate::handlers::api_keys::ApiKeyResponse,
            crate::handlers::api_keys::ApiKeyListResponse,
            crate::handlers::api_keys::DeleteApiKeyResponse,
            crate::handlers::api_keys::DurableGrantReceipt,
            crate::handlers::api_keys::DurableGrantListQuery,
            crate::handlers::api_keys::DurableGrantListResponse,
            crate::handlers::api_keys::ReauthorizeDurableGrantsRequest,
            crate::services::api_key_scope_service::EffectiveScopePlan,
            crate::services::api_key_scope_service::ScopePlanPrincipal,
            crate::services::api_key_scope_service::ScopePlanOwnerType,
            crate::services::api_key_scope_service::ScopePlanServiceGrant,
            crate::services::api_key_scope_service::ScopePlanNodeGrant,
            crate::services::api_key_scope_service::ScopePlanFreshness,
            crate::services::api_key_scope_service::ScopePlanFreshnessMode,
            crate::services::api_key_scope_service::ScopePlanPostCreationDrift,
            crate::services::api_key_scope_service::ScopePlanCompleteness,
            crate::services::api_key_scope_service::ScopePlanRouteCandidateBasis,
            crate::models::api_key::ApiKeyPurpose,
            crate::models::durable_operation_grant::DurableValueConstraint,
            crate::models::durable_operation_grant::DurableParameterConstraint,
            crate::models::durable_operation_grant::DurableBodyConstraint,
            crate::models::durable_operation_grant::DurableOperationConstraints,
            crate::models::durable_operation_grant::DurableUsageWindow,
            crate::models::durable_operation_grant::DurableReplayPolicy,
            crate::models::durable_operation_grant::DurableClientAuditBinding,
            crate::models::durable_operation_grant::DurableOperationSelection,
            crate::models::durable_operation_grant::DurableOperationPlan,
            // Billing
            crate::handlers::billing::BillingUsageResponse,
            crate::handlers::billing::BillingUsageRow,
            crate::handlers::billing::BillingUsageTotals,
            crate::handlers::billing::BillingReadOnlyBlock,
            crate::handlers::billing::TopUpHistoryResponse,
            crate::handlers::billing::TopUpHistoryEntry,
            crate::handlers::billing::InvoiceDownloadResponse,
            crate::handlers::billing::ProvisionWalletRequest,
            crate::handlers::billing::TopUpRequest,
            crate::handlers::billing::BillingWalletResponse,
            crate::handlers::billing::TopUpResponse,
            // Demo
            crate::handlers::demo::DemoResponse
        )
    ),
    tags(
        (name = "Docs", description = "NyxID API documentation endpoints"),
        (name = "Proxy Docs", description = "Downstream OpenAPI and AsyncAPI catalog endpoints"),
        (name = "Services", description = "Downstream service management (admin)"),
        (name = "Proxy", description = "Authenticated downstream service discovery"),
        (name = "SSH", description = "SSH certificate issuance and WebSocket tunnel endpoints"),
        (name = "AI Services", description = "Unified key management: auto-provisions endpoint, credential, and proxy routing from catalog or custom input"),
        (name = "Connect Links", description = "Hosted single-use service credential connection flows"),
        (name = "Catalog", description = "Read-only service catalog for users (admin-created services and providers)"),
        (name = "Endpoints", description = "User-managed target URLs"),
        (name = "External API Keys", description = "User's external API keys and credentials"),
        (name = "User Services", description = "User's proxy routing configuration"),
        (name = "API Keys", description = "NyxID API keys with service and node scope"),
        (name = "Billing", description = "Owner billing wallet and usage endpoints"),
        (name = "Demo", description = "First-success verification: returns 200 with no downstream call")
    )
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

        openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::new)
            .add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
    }
}
