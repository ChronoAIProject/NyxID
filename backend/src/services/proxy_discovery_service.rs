use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::errors::AppResult;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, legacy_http_service_type_filter,
};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::UserService;
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
use crate::services::{node_routing_service, user_service_service};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverySource {
    Catalog,
    Custom,
}

impl DiscoverySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyDiscoveryItem {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub service_category: String,
    pub connected: bool,
    pub requires_connection: bool,
    pub has_node_binding: bool,
    pub proxy_url: String,
    pub proxy_url_slug: String,
    pub docs_url: Option<String>,
    pub openapi_url: Option<String>,
    pub asyncapi_url: Option<String>,
    pub streaming_supported: bool,
    pub websocket_supported: bool,
    pub source: DiscoverySource,
}

#[derive(Debug, Clone)]
pub struct ProxyDiscoveryPage {
    pub services: Vec<ProxyDiscoveryItem>,
    pub custom_services: Vec<ProxyDiscoveryItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

fn build_proxy_urls(base_url: &str, id: &str, slug: &str) -> (String, String) {
    (
        format!("{base_url}/api/v1/proxy/{id}/{{path}}"),
        format!("{base_url}/api/v1/proxy/s/{slug}/{{path}}"),
    )
}

fn docs_urls(
    base_url: &str,
    id: &str,
    has_openapi: bool,
    has_asyncapi: bool,
) -> (Option<String>, Option<String>, Option<String>) {
    let docs_url = (has_openapi || has_asyncapi)
        .then(|| format!("{base_url}/api/v1/proxy/services/{id}/docs"));
    let openapi_url =
        has_openapi.then(|| format!("{base_url}/api/v1/proxy/services/{id}/openapi.json"));
    let asyncapi_url =
        has_asyncapi.then(|| format!("{base_url}/api/v1/proxy/services/{id}/asyncapi.json"));
    (docs_url, openapi_url, asyncapi_url)
}

fn catalog_item(
    service: &DownstreamService,
    base_url: &str,
    connected: bool,
    has_node_binding: bool,
) -> ProxyDiscoveryItem {
    let (proxy_url, proxy_url_slug) = build_proxy_urls(base_url, &service.id, &service.slug);
    let (docs_url, openapi_url, asyncapi_url) = docs_urls(
        base_url,
        &service.id,
        service.openapi_spec_url.is_some(),
        service.asyncapi_spec_url.is_some(),
    );

    ProxyDiscoveryItem {
        id: service.id.clone(),
        name: service.name.clone(),
        slug: service.slug.clone(),
        description: service.description.clone(),
        service_category: service.service_category.clone(),
        connected,
        requires_connection: service.requires_user_credential,
        has_node_binding,
        proxy_url,
        proxy_url_slug,
        docs_url,
        openapi_url,
        asyncapi_url,
        streaming_supported: service.streaming_supported,
        websocket_supported: service
            .capabilities
            .as_ref()
            .is_some_and(|c| c.supports_websocket),
        source: DiscoverySource::Catalog,
    }
}

fn custom_item(
    service: &UserService,
    endpoint: &UserEndpoint,
    base_url: &str,
) -> Option<ProxyDiscoveryItem> {
    endpoint.openapi_spec_url.as_ref()?;

    let name = if endpoint.label.trim().is_empty() {
        service.slug.clone()
    } else {
        endpoint.label.clone()
    };
    let (proxy_url, proxy_url_slug) = build_proxy_urls(base_url, &service.id, &service.slug);
    let (docs_url, openapi_url, asyncapi_url) = docs_urls(base_url, &service.id, true, false);

    Some(ProxyDiscoveryItem {
        id: service.id.clone(),
        name,
        slug: service.slug.clone(),
        description: None,
        service_category: "custom".to_string(),
        connected: true,
        requires_connection: false,
        has_node_binding: service
            .node_id
            .as_ref()
            .is_some_and(|node_id| !node_id.is_empty()),
        proxy_url,
        proxy_url_slug,
        docs_url,
        openapi_url,
        asyncapi_url,
        streaming_supported: false,
        websocket_supported: false,
        source: DiscoverySource::Custom,
    })
}

pub fn project_catalog_key(
    service: &DownstreamService,
    service_id: &str,
    slug: &str,
    base_url: &str,
    connected: bool,
    has_node_binding: bool,
) -> ProxyDiscoveryItem {
    let mut item = catalog_item(service, base_url, connected, has_node_binding);
    item.id = service_id.to_string();
    item.slug = slug.to_string();
    let (proxy_url, proxy_url_slug) = build_proxy_urls(base_url, service_id, slug);
    item.proxy_url = proxy_url;
    item.proxy_url_slug = proxy_url_slug;
    let (docs_url, openapi_url, asyncapi_url) = docs_urls(
        base_url,
        service_id,
        service.openapi_spec_url.is_some(),
        service.asyncapi_spec_url.is_some(),
    );
    item.docs_url = docs_url;
    item.openapi_url = openapi_url;
    item.asyncapi_url = asyncapi_url;
    item
}

pub fn project_custom_key(
    service: &UserService,
    endpoint: &UserEndpoint,
    base_url: &str,
) -> ProxyDiscoveryItem {
    custom_item(service, endpoint, base_url).unwrap_or_else(|| {
        let (proxy_url, proxy_url_slug) = build_proxy_urls(base_url, &service.id, &service.slug);
        ProxyDiscoveryItem {
            id: service.id.clone(),
            name: if endpoint.label.trim().is_empty() {
                service.slug.clone()
            } else {
                endpoint.label.clone()
            },
            slug: service.slug.clone(),
            description: None,
            service_category: "custom".to_string(),
            connected: true,
            requires_connection: false,
            has_node_binding: service
                .node_id
                .as_ref()
                .is_some_and(|node_id| !node_id.is_empty()),
            proxy_url,
            proxy_url_slug,
            docs_url: None,
            openapi_url: None,
            asyncapi_url: None,
            streaming_supported: false,
            websocket_supported: false,
            source: DiscoverySource::Custom,
        }
    })
}

pub async fn list_proxy_discovery(
    db: &mongodb::Database,
    user_id: &str,
    ws_manager: &crate::services::node_ws_manager::NodeWsManager,
    base_url: &str,
    page: u64,
    per_page: u64,
) -> AppResult<ProxyDiscoveryPage> {
    let page = page.max(1);
    let per_page = per_page.min(100);
    let offset = (page - 1) * per_page;

    let mut filter = doc! {
        "is_active": true,
        "service_category": { "$ne": "provider" },
    };
    filter.extend(legacy_http_service_type_filter());

    let total = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .count_documents(filter.clone())
        .await?;

    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
        .sort(doc! { "name": 1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    let service_ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    let connections: Vec<UserServiceConnection> = if service_ids.is_empty() {
        vec![]
    } else {
        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .find(doc! {
                "user_id": user_id,
                "service_id": { "$in": &service_ids },
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?
    };
    let connected_set: HashSet<&str> = connections.iter().map(|c| c.service_id.as_str()).collect();

    let bound_service_ids =
        node_routing_service::list_routable_service_ids(db, user_id, ws_manager).await?;
    let node_bound_set: HashSet<&str> = bound_service_ids.iter().map(|s| s.as_str()).collect();

    let services = services
        .iter()
        .map(|s| {
            catalog_item(
                s,
                base_url,
                connected_set.contains(s.id.as_str()),
                node_bound_set.contains(s.id.as_str()),
            )
        })
        .collect();

    let visible_user_services = user_service_service::list_user_services_with_sources(db, user_id)
        .await?
        .into_iter()
        .map(|entry| entry.service)
        .filter(|service| service.catalog_service_id.is_none() && service.service_type == "http")
        .collect::<Vec<_>>();

    let endpoint_ids: Vec<&str> = visible_user_services
        .iter()
        .map(|service| service.endpoint_id.as_str())
        .collect();
    let endpoints: Vec<UserEndpoint> = if endpoint_ids.is_empty() {
        vec![]
    } else {
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .find(doc! { "_id": { "$in": &endpoint_ids } })
            .await?
            .try_collect()
            .await?
    };
    let endpoint_by_id: HashMap<String, UserEndpoint> = endpoints
        .into_iter()
        .map(|endpoint| (endpoint.id.clone(), endpoint))
        .collect();

    let mut custom_services: Vec<ProxyDiscoveryItem> = visible_user_services
        .into_iter()
        .filter_map(|service| {
            let endpoint = endpoint_by_id.get(&service.endpoint_id)?;
            custom_item(&service, endpoint, base_url)
        })
        .collect();
    custom_services.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.slug.cmp(&right.slug))
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(ProxyDiscoveryPage {
        services,
        custom_services,
        total,
        page,
        per_page,
    })
}
