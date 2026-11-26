use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, OnceLock},
};

use axum::{
    extract::MatchedPath,
    http::{HeaderValue, Method},
};
use serde::Serialize;
use tokio::time::Instant;

const MAX_ENTRIES: usize = 20;

tokio::task_local! {
    static HTTP_ROUTE: Route;
}

#[derive(Clone)]
struct Route {
    template: Option<MatchedPath>,
    method: &'static str,
    service: Option<Arc<OnceLock<Service>>>,
}

pub(crate) async fn scope<F: Future>(
    template: Option<MatchedPath>,
    method: &Method,
    future: F,
) -> F::Output {
    let method = match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::CONNECT => "CONNECT",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::PATCH => "PATCH",
        _ => "OTHER",
    };
    let service = template
        .as_ref()
        .is_some_and(|route| service_route(route.as_str()))
        .then(|| Arc::new(OnceLock::new()));
    HTTP_ROUTE
        .scope(
            Route {
                template,
                method,
                service,
            },
            future,
        )
        .await
}

fn service_route(route: &str) -> bool {
    matches!(
        route.trim_end_matches('/'),
        "/api/v1/proxy/{service_id}"
            | "/api/v1/proxy/{service_id}/{*path}"
            | "/api/v1/proxy/s/{slug}"
            | "/api/v1/proxy/s/{slug}/{*path}"
            | "/public/s/{slug}"
            | "/public/s/{slug}/{*path}"
            | "/api/v1/llm/gateway/v1/{*path}"
            | "/api/v1/llm/{provider_slug}/v1/{*path}"
            | "/api/v1/public-nyxbot/models/{service_id}/{*path}"
            | "/api/v1/proxy/nyxbot-voice/{service_id}"
            | "/api/v1/proxy/nyxbot-voice/{service_id}/{*path}"
    )
}

#[derive(Clone)]
enum Service {
    Catalog(Arc<str>),
    Custom,
}

impl Service {
    fn as_str(&self) -> &str {
        match self {
            Self::Catalog(slug) => slug,
            Self::Custom => "custom",
        }
    }
}

/// Accept only the resolver's catalog identity, never its user-facing alias.
pub(crate) fn resolved_proxy(target: &crate::services::proxy_service::ProxyTarget) {
    record_service(
        target
            .catalog_slug
            .as_deref()
            .filter(|_| target.catalog_is_public),
    );
}

pub(crate) fn resolved_catalog(service: &crate::models::downstream_service::DownstreamService) {
    record_service(
        crate::services::catalog_service::is_public_unowned_discovery_entry(service)
            .then_some(service.slug.as_str()),
    );
}

fn record_service(catalog_slug: Option<&str>) {
    let _ = HTTP_ROUTE.try_with(|route| {
        if let Some(service) = &route.service {
            service.get_or_init(|| match catalog_slug {
                Some(slug) if !slug.is_empty() && uuid::Uuid::try_parse(slug).is_err() => {
                    Service::Catalog(Arc::from(slug))
                }
                _ => Service::Custom,
            });
        }
    });
}

pub(crate) struct ServiceSelection(Option<Arc<OnceLock<Service>>>);

impl ServiceSelection {
    pub(crate) fn publish(self) {
        if let Some(label) = self.0.as_ref().and_then(|service| service.get()) {
            let _ = HTTP_ROUTE.try_with(|route| {
                if let Some(service) = &route.service {
                    let _ = service.set(label.clone());
                }
            });
        }
    }
}

/// A failed pool candidate must not label the response from a later candidate.
/// The outer request stays unresolved until the pool selects its response.
pub(crate) async fn candidate<F: Future>(future: F) -> (F::Output, ServiceSelection) {
    let Ok(mut route) = HTTP_ROUTE.try_with(Clone::clone) else {
        return (future.await, ServiceSelection(None));
    };
    route.service = route.service.map(|_| Arc::new(OnceLock::new()));
    let selection = ServiceSelection(route.service.clone());
    (HTTP_ROUTE.scope(route, future).await, selection)
}

pub(super) fn content_type_family(value: Option<&HeaderValue>) -> &'static str {
    let media = value
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if media.eq_ignore_ascii_case("text/event-stream") {
        "event-stream"
    } else if media.eq_ignore_ascii_case("application/json")
        || media.eq_ignore_ascii_case("text/json")
        || media.split_once('/').is_some_and(|(_, subtype)| {
            subtype
                .rsplit_once('+')
                .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("json"))
        })
    {
        "json"
    } else {
        "other"
    }
}

struct Unit {
    route: Route,
    kind: &'static str,
    content_type: Option<&'static str>,
    started: Instant,
}

enum Slot {
    Open(Unit),
    Free(Option<usize>),
}

/// Reuses the concurrency high-water allocation. Vacant slots retain no route.
#[derive(Default)]
pub(super) struct Registry {
    slots: Vec<Slot>,
    free: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct OpenHttp {
    pub entries: Vec<Entry>,
    pub other_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub route: String,
    pub method: &'static str,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<&'static str>,
    pub count: usize,
    pub oldest_age_secs: u64,
}

impl Registry {
    pub(super) fn insert(
        &mut self,
        kind: &'static str,
        content_type: Option<&'static str>,
    ) -> usize {
        let unit = Slot::Open(Unit {
            route: HTTP_ROUTE.try_with(Clone::clone).unwrap_or(Route {
                template: None,
                method: "OTHER",
                service: None,
            }),
            kind,
            content_type: (kind == "stream").then_some(content_type.unwrap_or("other")),
            started: Instant::now(),
        });
        if let Some(index) = self.free {
            let Slot::Free(next) = std::mem::replace(&mut self.slots[index], unit) else {
                unreachable!("free-list slot must be vacant");
            };
            self.free = next;
            index
        } else {
            self.slots.push(unit);
            self.slots.len() - 1
        }
    }

    pub(super) fn remove(&mut self, index: usize) {
        self.slots[index] = Slot::Free(self.free);
        self.free = Some(index);
    }

    pub(super) fn snapshot(&self) -> OpenHttp {
        let now = Instant::now();
        let mut groups = BTreeMap::<_, (usize, u64)>::new();
        for slot in &self.slots {
            let Slot::Open(unit) = slot else { continue };
            let key = (
                unit.route
                    .template
                    .as_ref()
                    .map_or("<unmatched>", MatchedPath::as_str),
                unit.route.method,
                unit.kind,
                unit.route
                    .service
                    .as_ref()
                    .and_then(|service| service.get())
                    .map(Service::as_str),
                unit.content_type,
            );
            let (count, age) = groups.entry(key).or_default();
            *count += 1;
            *age = (*age).max(now.saturating_duration_since(unit.started).as_secs());
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_unstable_by(|(a, (a_count, _)), (b, (b_count, _))| {
            b_count.cmp(a_count).then_with(|| a.cmp(b))
        });
        OpenHttp {
            other_count: groups
                .iter()
                .skip(MAX_ENTRIES)
                .map(|(_, (count, _))| count)
                .sum(),
            entries: groups
                .into_iter()
                .take(MAX_ENTRIES)
                .map(
                    |((route, method, kind, service, content_type), (count, oldest_age_secs))| {
                        Entry {
                            route: route.to_owned(),
                            method,
                            kind,
                            service: service.map(str::to_owned),
                            content_type,
                            count,
                            oldest_age_secs,
                        }
                    },
                )
                .collect(),
        }
    }
}
