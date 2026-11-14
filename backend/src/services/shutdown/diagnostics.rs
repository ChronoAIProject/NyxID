use std::{collections::BTreeMap, future::Future};

use axum::{extract::MatchedPath, http::Method};
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
    HTTP_ROUTE.scope(Route { template, method }, future).await
}

struct Unit {
    route: Route,
    kind: &'static str,
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
    pub count: usize,
    pub oldest_age_secs: u64,
}

impl Registry {
    pub(super) fn insert(&mut self, kind: &'static str) -> usize {
        let unit = Slot::Open(Unit {
            route: HTTP_ROUTE.try_with(Clone::clone).unwrap_or(Route {
                template: None,
                method: "OTHER",
            }),
            kind,
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
                .map(|((route, method, kind), (count, oldest_age_secs))| Entry {
                    route: route.to_owned(),
                    method,
                    kind,
                    count,
                    oldest_age_secs,
                })
                .collect(),
        }
    }
}
