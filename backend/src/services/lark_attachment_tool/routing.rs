//! Compatibility repair for two platform-authored bot paths. The stored path
//! remains the logical operation identity used by scopes and approval digests.
//! Only a trusted typed catalog call can obtain this in-process wire capability.
use super::*;
use crate::services::{
    destination_routing,
    proxy_authorization::{CanonicalPath, rule_forwarding_path},
    proxy_service::ProxyTarget,
};

pub(crate) struct BotRoute {
    catalog_id: String,
    slug: String,
    origin: String,
    method: String,
    logical: String,
    wire: String,
}

impl BotRoute {
    pub(crate) fn issue(
        catalog: &DownstreamService,
        endpoint: &McpToolEndpoint,
        path: &str,
    ) -> AppResult<Option<Self>> {
        if !matches!(catalog.slug.as_str(), "api-lark-bot" | "api-feishu-bot")
            || !matches!(endpoint.name.as_str(), FIELDS_OPERATION | UPLOAD_OPERATION)
            || catalog.created_by != "system"
            || catalog.tools_manager_created
        {
            return Ok(None);
        }
        // Freeze the known-bad logical identities, not any future path that a
        // later compiled catalog happens to assign to these operation names.
        let known_bad = match endpoint.name.as_str() {
            FIELDS_OPERATION => {
                endpoint.method == "GET"
                    && endpoint.path == "/bitable/v1/apps/{app_token}/tables/{table_id}/fields"
            }
            UPLOAD_OPERATION => endpoint.method == "POST" && endpoint.path == UPLOAD_PATH,
            _ => false,
        };
        if !known_bad {
            return Ok(None);
        }
        let origin = catalog.base_url.trim_end_matches('/');
        let expected_origin = if catalog.slug == "api-lark-bot" {
            "https://open.larksuite.com"
        } else {
            "https://open.feishu.cn"
        };
        let allowed_origin = origin == expected_origin;
        #[cfg(test)]
        let allowed_origin = allowed_origin
            || url::Url::parse(origin).is_ok_and(|url| {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.path() == "/"
                    && url.query().is_none()
                    && url.fragment().is_none()
            });
        if !allowed_origin {
            return Ok(None);
        }
        let spec = catalog_spec_registry::spec_for_slug(&catalog.slug)
            .ok_or_else(|| Reason::UntrustedContract.error())?;
        let Some(compiled) = catalog_spec_sync::destination_endpoint_inputs(catalog, &spec)?
            .into_iter()
            .find(|row| row.name == endpoint.name)
        else {
            return Ok(None);
        };
        let expected = McpToolEndpoint {
            name: compiled.name,
            method: compiled.method,
            path: compiled.path,
            target_id: compiled.target_id,
            parameters: compiled.parameters,
            request_body_schema: compiled.request_body_schema,
            request_content_type: compiled.request_content_type,
            request_body_required: compiled.request_body_required,
            response: compiled.response,
            async_operation: compiled.async_operation,
            ..Default::default()
        };
        if inputs(endpoint) != inputs(&expected) || endpoint.response != expected.response {
            return Ok(None);
        }
        let canonical = CanonicalPath::from_mcp_built(path)?;
        let rule = crate::models::downstream_service::ProxyOperationRule {
            method: expected.method.clone(),
            path_template: expected.path,
            ..Default::default()
        };
        let logical = rule_forwarding_path(&rule, &expected.method, &canonical)
            .ok_or_else(|| Reason::UntrustedContract.error())?;
        if logical.trim_start_matches('/') != path.trim_start_matches('/') {
            return Err(Reason::UntrustedContract.error());
        }
        let logical = logical.trim_start_matches('/').to_owned();
        Ok(Some(Self {
            catalog_id: catalog.id.clone(),
            slug: catalog.slug.clone(),
            origin: origin.into(),
            method: expected.method,
            wire: format!("open-apis/{logical}"),
            logical,
        }))
    }

    /// Run after ordinary logical-effect authorization and before URL assembly.
    /// Neither delegated path substitution nor a changed recipient may consume it.
    pub(crate) fn forwarding_path<'a>(
        &'a self,
        target: &ProxyTarget,
        method: &str,
        path: &str,
        prepared_path: &str,
    ) -> AppResult<&'a str> {
        if target.service.id != self.catalog_id
            || target.catalog_slug.as_deref() != Some(&self.slug)
            || target.base_url.trim_end_matches('/') != self.origin
            || method != self.method
            || path.trim_start_matches('/') != self.logical
            || prepared_path.trim_start_matches('/') != self.logical
        {
            return Err(Reason::DestinationChanged.error());
        }
        for path in [&self.logical, &self.wire] {
            let canonical = CanonicalPath::from_mcp_built(path)?;
            let (forwarding, selected) =
                destination_routing::select_target(&target.service, method, &canonical)?;
            if selected != target.target_id || forwarding.trim_start_matches('/') != path {
                return Err(Reason::DestinationChanged.error());
            }
        }
        Ok(&self.wire)
    }
}

pub(crate) async fn for_call(
    db: &Database,
    target: &ProxyTarget,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
    path: &str,
) -> AppResult<Option<BotRoute>> {
    if !matches!(
        target.catalog_slug.as_deref(),
        Some("api-lark-bot" | "api-feishu-bot")
    ) || !matches!(endpoint.name.as_str(), FIELDS_OPERATION | UPLOAD_OPERATION)
        || service.is_generic_proxy
        || service
            .durable_endpoint_metadata
            .get(&endpoint.endpoint_id)
            .is_none_or(|metadata| !metadata.catalog_contract || metadata.operation_generation <= 0)
    {
        return Ok(None);
    }
    let catalog = db
        .collection::<DownstreamService>("downstream_services")
        .find_one(doc! {"_id":&target.service.id,"is_active":true})
        .await?
        .ok_or_else(|| Reason::CatalogUnavailable.error())?;
    BotRoute::issue(&catalog, endpoint, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;

    fn catalog(slug: &str) -> DownstreamService {
        let mut catalog = dummy_service();
        catalog.slug = slug.into();
        catalog.created_by = "system".into();
        catalog.base_url = if slug == "api-feishu-bot" {
            "https://open.feishu.cn"
        } else {
            "https://open.larksuite.com"
        }
        .into();
        catalog
    }
    fn endpoint(catalog: &DownstreamService, name: &str) -> McpToolEndpoint {
        let row = catalog_spec_sync::destination_endpoint_inputs(
            catalog,
            &catalog_spec_registry::spec_for_slug(&catalog.slug).unwrap(),
        )
        .unwrap()
        .into_iter()
        .find(|row| row.name == name)
        .unwrap();
        McpToolEndpoint {
            name: row.name,
            method: row.method,
            path: row.path,
            parameters: row.parameters,
            target_id: row.target_id,
            request_body_schema: row.request_body_schema,
            request_content_type: row.request_content_type,
            request_body_required: row.request_body_required,
            response: row.response,
            async_operation: row.async_operation,
            ..Default::default()
        }
    }
    fn target(catalog: &DownstreamService) -> ProxyTarget {
        ProxyTarget {
            catalog_slug: Some(catalog.slug.clone()),
            execution_request: None,
            execution_permission: None,
            credential_deferred: false,
            workspace_destinations_pending: false,
            target_id: None,
            base_url: catalog.base_url.clone(),
            auth_method: "token_exchange".into(),
            auth_key_name: "Authorization".into(),
            credential: zeroize::Zeroizing::new(String::new()),
            service: catalog.clone(),
            catalog_default_headers: vec![],
            user_service_default_headers: vec![],
            ws_frame_injections: vec![],
            connection_id: None,
        }
    }

    #[test]
    fn lark_attachment_routing_is_exact_and_keeps_every_contract_digest() {
        for slug in ["api-lark-bot", "api-feishu-bot"] {
            let catalog = catalog(slug);
            for name in [FIELDS_OPERATION, UPLOAD_OPERATION] {
                let endpoint = endpoint(&catalog, name);
                let path = endpoint
                    .path
                    .replace("{app_token}", "base")
                    .replace("{table_id}", "table");
                let digest = super::super::super::mcp_service::endpoint_contract_digest(&endpoint);
                let route = BotRoute::issue(&catalog, &endpoint, &path)
                    .unwrap()
                    .unwrap();
                let target = target(&catalog);
                assert_eq!(
                    route
                        .forwarding_path(
                            &target,
                            &endpoint.method,
                            &path,
                            path.trim_start_matches('/')
                        )
                        .unwrap(),
                    format!("open-apis/{}", path.trim_start_matches('/'))
                );
                assert_eq!(
                    digest,
                    super::super::super::mcp_service::endpoint_contract_digest(&endpoint)
                );
                for variant in ["origin", "catalog", "slug", "target", "policy"] {
                    let mut changed = self::target(&catalog);
                    match variant {
                        "origin" => changed.base_url = "https://attacker.invalid".into(),
                        "catalog" => changed.service.id.push('x'),
                        "slug" => changed.catalog_slug = Some("custom".into()),
                        "target" => changed.target_id = Some("other".into()),
                        _ => {
                            changed.service.proxy_operation_policy =
                                Some(crate::models::downstream_service::ProxyOperationPolicy {
                                    rules: vec![
                                        crate::models::downstream_service::ProxyOperationRule {
                                            method: endpoint.method.clone(),
                                            path_template: endpoint.path.clone(),
                                            ..Default::default()
                                        },
                                    ],
                                })
                        }
                    }
                    assert!(
                        route
                            .forwarding_path(&changed, &endpoint.method, &path, &path)
                            .is_err(),
                        "{variant}"
                    );
                }
                assert!(
                    route
                        .forwarding_path(&target, "DELETE", &path, &path)
                        .is_err()
                );
                assert!(
                    route
                        .forwarding_path(&target, &endpoint.method, &path, "substituted/path")
                        .is_err()
                );
                let mut changed = endpoint.clone();
                changed.path = format!("/open-apis{}", changed.path);
                assert!(
                    BotRoute::issue(&catalog, &changed, &path)
                        .unwrap()
                        .is_none()
                );
                let mut changed = endpoint.clone();
                changed.request_body_required ^= true;
                assert!(
                    BotRoute::issue(&catalog, &changed, &path)
                        .unwrap()
                        .is_none()
                );
                let mut changed = catalog.clone();
                changed.created_by = "admin".into();
                assert!(
                    BotRoute::issue(&changed, &endpoint, &path)
                        .unwrap()
                        .is_none()
                );
                changed.created_by = "system".into();
                changed.base_url.push_str("/open-apis");
                assert!(
                    BotRoute::issue(&changed, &endpoint, &path)
                        .unwrap()
                        .is_none()
                );
            }
            let create = endpoint(&catalog, CREATE_OPERATION);
            assert!(
                BotRoute::issue(
                    &catalog,
                    &create,
                    "/open-apis/bitable/v1/apps/base/tables/table/records"
                )
                .unwrap()
                .is_none()
            );
        }
    }

    #[tokio::test]
    async fn lark_attachment_routing_requires_explicit_capability_at_the_wire() {
        use crate::services::{
            cloud_response_cache::CloudResponseCache,
            provider_token_exchange_service::TokenExchangeCache, proxy_service as proxy,
        };
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut catalog = catalog("api-lark-bot");
        catalog.base_url = server.uri();
        let endpoint = endpoint(&catalog, FIELDS_OPERATION);
        let path = "/bitable/v1/apps/base/tables/table/fields";
        let route = BotRoute::issue(&catalog, &endpoint, path).unwrap().unwrap();
        let mut target = target(&catalog);
        target.auth_method = "none".into();
        for capability in [None, Some(&route)] {
            proxy::forward_request_with_resource_proof(
                proxy::no_redirect_http_client(),
                &target,
                reqwest::Method::GET,
                path,
                Some("page_size=100"),
                reqwest::header::HeaderMap::new(),
                proxy::ProxyBody::Buffered(None),
                vec![],
                vec![],
                None,
                &TokenExchangeCache::new(),
                &CloudResponseCache::new(0),
                vec![],
                crate::services::billing::route_inventory::internal_node_dispatch_permit(),
                None,
                None,
                None,
                None,
                capability,
            )
            .await
            .unwrap();
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url.path(), path);
        assert_eq!(requests[1].url.path(), format!("/open-apis{path}"));
        for request in requests {
            assert_eq!(request.url.query(), Some("page_size=100"));
            assert!(request.body.is_empty());
        }
    }
}
