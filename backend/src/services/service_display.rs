use std::collections::{BTreeMap, BTreeSet};

use futures::TryStreamExt;
use mongodb::{ClientSession, Database, bson::doc};
use serde::Deserialize;

use super::api_key_scope_service::{
    active_service_owner_access_in_session, actor_can_scope_service,
};
use crate::{
    errors::AppResult,
    models::{
        user_endpoint::COLLECTION_NAME as ENDPOINTS, user_service::COLLECTION_NAME as SERVICES,
    },
};

pub(super) struct ServiceDisplay {
    pub label: Option<String>,
    pub slug: Option<String>,
    pub catalog_service_id: Option<String>,
    pub disabled: bool,
}

/// Project labels for selected services without reading credential material.
/// The caller supplies the actor and any source-level service ceiling.
pub(super) async fn load_in_session(
    db: &Database,
    user: &str,
    ids: &[&str],
    allow_org_access: bool,
    session: &mut ClientSession,
) -> AppResult<BTreeMap<String, ServiceDisplay>> {
    let mut displays = BTreeMap::new();
    if ids.is_empty() {
        return Ok(displays);
    }
    #[derive(Deserialize)]
    struct ServiceEndpoint {
        #[serde(rename = "_id")]
        id: String,
        user_id: String,
        endpoint_id: String,
        slug: String,
        catalog_service_id: Option<String>,
        is_active: bool,
        #[serde(default)]
        admin_only: bool,
    }
    #[derive(Deserialize)]
    struct EndpointLabel {
        #[serde(rename = "_id")]
        id: String,
        user_id: String,
        label: Option<String>,
    }
    let mut cursor = db
        .collection::<ServiceEndpoint>(SERVICES)
        .find(doc! {"_id": {"$in": ids}, "deleted_at": null})
        .projection(doc! {"_id": 1, "user_id": 1, "endpoint_id": 1, "is_active": 1, "admin_only": 1, "slug": 1, "catalog_service_id": 1})
        .session(&mut *session)
        .await?;
    let services: Vec<ServiceEndpoint> = cursor.stream(&mut *session).try_collect().await?;

    let owners = services
        .iter()
        .map(|service| service.user_id.as_str())
        .filter(|owner| allow_org_access || *owner == user)
        .collect();
    let access = active_service_owner_access_in_session(db, user, &owners, session).await?;
    let authorized: Vec<_> = services
        .into_iter()
        .filter(|service| {
            access.get(&service.user_id).is_some_and(|access| {
                actor_can_scope_service(access, &service.id)
                    && (!service.admin_only || access.can_write())
            })
        })
        .collect();
    if authorized.is_empty() {
        return Ok(displays);
    }
    let mut endpoints_by_owner = BTreeMap::<_, BTreeSet<_>>::new();
    for service in &authorized {
        endpoints_by_owner
            .entry(service.user_id.as_str())
            .or_default()
            .insert(service.endpoint_id.as_str());
    }
    let endpoint_filters: Vec<_> = endpoints_by_owner
        .into_iter()
        .map(|(owner, ids)| {
            doc! {"user_id": owner, "_id": {"$in": ids.into_iter().collect::<Vec<_>>()}}
        })
        .collect();
    let mut cursor = db
        .collection::<EndpointLabel>(ENDPOINTS)
        .find(doc! {"$or": endpoint_filters, "deleted_at": null})
        .projection(doc! {"_id": 1, "user_id": 1, "label": 1})
        .session(&mut *session)
        .await?;
    let labels: Vec<EndpointLabel> = cursor.stream(&mut *session).try_collect().await?;
    let endpoints: BTreeMap<_, _> = labels
        .iter()
        .map(|row| ((row.user_id.as_str(), row.id.as_str()), row))
        .collect();
    for service in authorized {
        let endpoint = endpoints.get(&(service.user_id.as_str(), service.endpoint_id.as_str()));
        // A legacy deletion removed the endpoint without marking deleted_at.
        if !service.is_active && endpoint.is_none() {
            continue;
        }
        let label = endpoint
            .and_then(|row| row.label.as_ref())
            .filter(|label| {
                let label = label.trim();
                !label.is_empty() && label != "—" && !label.eq_ignore_ascii_case(&service.id)
            })
            .cloned();

        let slug = service.slug.trim();
        let slug = (!slug.is_empty() && slug != "—" && !slug.eq_ignore_ascii_case(&service.id))
            .then(|| slug.to_owned());
        displays.insert(
            service.id,
            ServiceDisplay {
                label,
                slug,
                catalog_service_id: endpoint.and(service.catalog_service_id),
                disabled: !service.is_active,
            },
        );
    }
    Ok(displays)
}
