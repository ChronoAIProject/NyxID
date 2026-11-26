use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Bson, doc},
};
use serde::Deserialize;

use crate::{
    errors::AppResult,
    models::{downstream_service, provider_config, service_endpoint, user_api_key, user_endpoint},
    services::{
        org_service, platform_key_service, scope_catalog,
        user_service_service::{self, CredentialSource},
    },
};

const READ_DEADLINE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    #[serde(rename = "oauth")]
    OAuth,
    ApiKey,
    Platform,
    NodeManaged,
    Other,
    Unknown,
}

#[derive(Debug)]
pub struct ServiceDisclosure {
    pub service_id: String,
    pub source: CredentialSource,
    pub credential_kind: CredentialKind,
    pub upstream_scopes: Option<Vec<UpstreamScope>>,
    pub known_operations: Option<OperationCounts>,
    pub model_calls: Option<bool>,
    pub binary_upload: Option<bool>,
}

#[derive(Debug, PartialEq, serde::Serialize, utoipa::ToSchema)]
pub struct UpstreamScope {
    pub scope: String,
    pub known: bool,
    pub label: Option<String>,
    pub description: Option<String>,
    pub sensitive: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OperationCounts {
    pub read: u32,
    pub write: u32,
    pub delete: u32,
    pub unclassified: u32,
}

#[derive(Deserialize)]
struct KeyMetadata {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    credential_type: String,
    status: String,
    token_scopes: Option<String>,
    provider_config_id: Option<String>,
}

#[derive(Deserialize)]
struct ProviderMetadata {
    #[serde(rename = "_id")]
    id: String,
    slug: String,
}

#[derive(Deserialize)]
struct MountedEndpoint {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
}

#[derive(Deserialize)]
struct CatalogMetadata {
    #[serde(rename = "_id")]
    id: String,
    inference: Option<downstream_service::ServiceInference>,
    capabilities: Option<downstream_service::ServiceCapabilities>,
}

#[derive(Deserialize)]
struct CatalogOperationCounts {
    #[serde(rename = "_id")]
    catalog_id: String,
    #[serde(flatten)]
    counts: OperationCounts,
}

pub async fn list(
    db: &Database,
    actor_id: &str,
    service_ids: &[String],
) -> AppResult<Vec<ServiceDisclosure>> {
    if service_ids.is_empty() {
        return Ok(Vec::new());
    }
    let memberships = org_service::list_memberships_for_member(db, actor_id, false).await?;
    let mut services = user_service_service::list_user_services_with_sources_selected(
        db,
        actor_id,
        false,
        false,
        &memberships,
        Some(service_ids),
    )
    .await?;
    services.retain(|row| !matches!(row.source, CredentialSource::Org { allowed: false, .. }));
    if services.is_empty() {
        return Ok(Vec::new());
    }

    let key_ids: HashSet<_> = services
        .iter()
        .filter(|row| {
            platform_key_service::binding(&row.service) == "user"
                && row.service.auth_method != "none"
                && row.service.node_id.is_none()
        })
        .filter_map(|row| row.service.api_key_id.as_deref())
        .collect();
    let catalog_ids: HashSet<_> = services
        .iter()
        .filter_map(|row| row.service.catalog_service_id.as_deref())
        .collect();
    let key_ids: Vec<_> = key_ids.into_iter().collect();
    let catalog_ids: Vec<_> = catalog_ids.into_iter().collect();
    let endpoint_ids: Vec<_> = services
        .iter()
        .filter(|row| row.service.catalog_service_id.is_some())
        .map(|row| row.service.endpoint_id.as_str())
        .collect();

    let (keys, catalog, mounted_endpoints): (
        Vec<KeyMetadata>,
        Vec<CatalogMetadata>,
        Vec<MountedEndpoint>,
    ) = tokio::try_join!(
        async {
            db.collection::<KeyMetadata>(user_api_key::COLLECTION_NAME)
                .find(doc! { "_id": { "$in": &key_ids } })
                .projection(doc! {
                    "user_id": 1, "credential_type": 1, "status": 1, "token_scopes": 1,
                    "provider_config_id": 1,
                })
                .max_time(READ_DEADLINE)
                .await?
                .try_collect()
                .await
        },
        async {
            db.collection::<CatalogMetadata>(downstream_service::COLLECTION_NAME)
                .find(doc! { "_id": { "$in": &catalog_ids }, "is_active": true })
                .projection(doc! { "inference": 1, "capabilities": 1 })
                .max_time(READ_DEADLINE)
                .await?
                .try_collect()
                .await
        },
        async {
            if endpoint_ids.is_empty() {
                return Ok(Vec::new());
            }
            db.collection::<MountedEndpoint>(user_endpoint::COLLECTION_NAME)
                .find(doc! {
                    "_id": { "$in": &endpoint_ids },
                    "openapi_spec_url": { "$type": "string" },
                })
                .projection(doc! { "user_id": 1 })
                .max_time(READ_DEADLINE)
                .await?
                .try_collect()
                .await
        },
    )?;
    let mounted_endpoints: HashMap<_, _> = mounted_endpoints
        .into_iter()
        .map(|endpoint| (endpoint.id, endpoint.user_id))
        .collect();
    let keys: HashMap<_, _> = keys.into_iter().map(|key| (key.id.clone(), key)).collect();
    let catalog: HashMap<_, _> = catalog.iter().map(|row| (row.id.as_str(), row)).collect();
    let catalog_ids: Vec<_> = catalog.keys().copied().collect();
    let provider_ids: HashSet<_> = services
        .iter()
        .filter_map(|row| {
            let key = keys.get(row.service.api_key_id.as_deref()?)?;
            (key.user_id == row.service.user_id && key.credential_type == "oauth2")
                .then_some(key.provider_config_id.as_deref())
                .flatten()
        })
        .collect();
    let provider_ids: Vec<_> = provider_ids.into_iter().collect();

    let (providers, operations): (Vec<ProviderMetadata>, Vec<CatalogOperationCounts>) = tokio::try_join!(
        async {
            db.collection::<ProviderMetadata>(provider_config::COLLECTION_NAME)
                .find(doc! { "_id": { "$in": provider_ids } })
                .projection(doc! { "slug": 1 })
                .max_time(READ_DEADLINE)
                .await?
                .try_collect()
                .await
        },
        async {
            // Missing publication is the model's legacy Published default.
            // Group at the database so spec size does not affect reply size.
            db.collection::<mongodb::bson::Document>(service_endpoint::COLLECTION_NAME)
                .aggregate([
                    doc! { "$match": {
                        "service_id": { "$in": catalog_ids },
                        "is_active": true,
                        "$or": [
                            { "publication": "published" },
                            { "publication": { "$exists": false } },
                        ],
                    }},
                    doc! { "$group": {
                        "_id": "$service_id",
                        "read": { "$sum": { "$cond": [{ "$eq": ["$risk", "read"] }, 1, 0] }},
                        "write": { "$sum": { "$cond": [{ "$and": [
                            { "$eq": ["$risk", "write"] }, { "$ne": ["$method", "DELETE"] },
                        ] }, 1, 0] }},
                        "delete": { "$sum": { "$cond": [{ "$and": [
                            { "$eq": ["$risk", "write"] }, { "$eq": ["$method", "DELETE"] },
                        ] }, 1, 0] }},
                        "unclassified": { "$sum": { "$cond": [
                            { "$in": [{ "$ifNull": ["$risk", Bson::Null] }, ["read", "write"]] },
                            0, 1,
                        ] }},
                    }},
                ])
                .max_time(READ_DEADLINE)
                .with_type::<CatalogOperationCounts>()
                .await?
                .try_collect()
                .await
        },
    )?;
    let scope_catalogs: HashMap<_, _> = providers
        .into_iter()
        .filter_map(|row| scope_catalog::for_provider(&row.slug).map(|scopes| (row.id, scopes)))
        .collect();
    let operations: HashMap<_, _> = operations
        .into_iter()
        .map(|row| (row.catalog_id, row.counts))
        .collect();

    Ok(services
        .into_iter()
        .map(|row| {
            let service = row.service;
            let key = service
                .api_key_id
                .as_deref()
                .and_then(|id| keys.get(id))
                .filter(|key| key.user_id == service.user_id);
            let credential_kind = if service.auth_method == "none" {
                // Catalog routing and exchange can also introduce credentials.
                CredentialKind::Unknown
            } else if service.node_id.is_some() {
                CredentialKind::NodeManaged
            } else if platform_key_service::binding(&service) == "platform" {
                CredentialKind::Platform
            } else {
                match key.map(|key| key.credential_type.as_str()) {
                    Some("oauth2") => CredentialKind::OAuth,
                    Some("api_key") => CredentialKind::ApiKey,
                    Some("node_managed") => CredentialKind::NodeManaged,
                    Some("bearer" | "basic" | "ssh_certificate") => CredentialKind::Other,
                    _ => CredentialKind::Unknown,
                }
            };
            let upstream_scopes = key
                .filter(|key| {
                    credential_kind == CredentialKind::OAuth
                        && matches!(key.status.as_str(), "active" | "expired")
                })
                .and_then(|key| {
                    let catalog = key
                        .provider_config_id
                        .as_ref()
                        .and_then(|id| scope_catalogs.get(id))
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    describe_scopes(key.token_scopes.as_deref()?, catalog)
                });
            let catalog_id = service.catalog_service_id.as_deref();
            let metadata = catalog_id.and_then(|id| catalog.get(id));
            // A mounted spec can replace the catalog operation inventory for
            // this connection. Disclosure leaves its operation counts unknown.
            let has_mounted_spec =
                mounted_endpoints.get(&service.endpoint_id) == Some(&service.user_id);
            let known_operations = catalog_id
                .filter(|_| !has_mounted_spec)
                .and_then(|id| operations.get(id))
                .cloned();
            ServiceDisclosure {
                service_id: service.id,
                source: row.source,
                credential_kind,
                upstream_scopes,
                known_operations,
                model_calls: metadata
                    .and_then(|row| row.inference.as_ref())
                    .map(|_| true),
                binary_upload: metadata
                    .and_then(|row| row.capabilities.as_ref())
                    .and_then(|caps| caps.supports_proxy_binary_upload.then_some(true)),
            }
        })
        .collect())
}

fn describe_scopes(
    recorded: &str,
    catalog: &[scope_catalog::ScopeCatalogEntry],
) -> Option<Vec<UpstreamScope>> {
    // A provider-side scope is an opaque printable token. Bound malformed or
    // oversized legacy values without misrepresenting a partial list as complete.
    if recorded.len() > 8192 {
        return None;
    }
    let mut seen = HashSet::new();
    let scopes = recorded
        .split_ascii_whitespace()
        .filter(|scope| seen.insert(*scope))
        .map(|scope| {
            let entry = catalog.iter().find(|entry| entry.scope == scope);
            UpstreamScope {
                scope: scope.to_owned(),
                known: entry.is_some(),
                label: entry.map(|entry| entry.label.clone()),
                description: entry.map(|entry| entry.description.clone()),
                sensitive: entry.map(|entry| entry.sensitive),
            }
        })
        .collect::<Vec<_>>();
    (!scopes.is_empty()
        && scopes.len() <= 100
        && scopes.iter().all(|scope| {
            scope
                .scope
                .bytes()
                .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
        }))
    .then_some(scopes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{org_membership, user_service};
    use crate::test_utils::connect_test_database;
    use mongodb::bson::{DateTime, Document};
    use uuid::Uuid;

    #[test]
    fn consent_disclosure_maps_only_recorded_permissions() {
        let catalog = scope_catalog::for_provider("github").unwrap();
        let scopes = describe_scopes("repo provider:custom repo", &catalog).unwrap();
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].scope, "repo");
        assert!(scopes[0].known);
        assert_eq!(scopes[0].sensitive, Some(true));
        assert_eq!(
            scopes[1],
            UpstreamScope {
                scope: "provider:custom".into(),
                known: false,
                label: None,
                description: None,
                sensitive: None,
            }
        );
        assert!(describe_scopes(" ", &catalog).is_none());
        assert!(describe_scopes("bad\"scope", &catalog).is_none());
    }

    #[tokio::test]
    async fn consent_disclosure_omits_catalog_counts_for_mounted_specs() {
        let Some(db) = connect_test_database("oauth_consent_disclosure_mounted_spec").await else {
            return;
        };
        let id = || Uuid::new_v4().to_string();
        let actor = id();
        let catalog_id = id();
        let catalog_service_id = id();
        let mounted_service_id = id();
        for (service_id, spec_url) in [
            (&catalog_service_id, None),
            (
                &mounted_service_id,
                Some("https://example.invalid/spec.json"),
            ),
        ] {
            let endpoint_id = id();
            let mut endpoint = doc! { "_id": &endpoint_id, "user_id": &actor };
            if let Some(spec_url) = spec_url {
                endpoint.insert("openapi_spec_url", spec_url);
            }
            db.collection::<Document>(user_endpoint::COLLECTION_NAME)
                .insert_one(endpoint)
                .await
                .unwrap();
            db.collection::<Document>(user_service::COLLECTION_NAME)
                .insert_one(doc! {
                    "_id": service_id, "user_id": &actor, "slug": service_id,
                    "endpoint_id": &endpoint_id, "catalog_service_id": &catalog_id,
                    "is_active": true, "auth_method": "none", "auth_key_name": "",
                    "created_at": DateTime::now(), "updated_at": DateTime::now(),
                })
                .await
                .unwrap();
        }
        db.collection::<Document>(downstream_service::COLLECTION_NAME)
            .insert_one(doc! { "_id": &catalog_id, "is_active": true })
            .await
            .unwrap();
        db.collection::<Document>(service_endpoint::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": id(), "service_id": &catalog_id, "is_active": true,
                "publication": "published", "method": "GET", "risk": "read",
            })
            .await
            .unwrap();

        let rows = list(
            &db,
            &actor,
            &[catalog_service_id.clone(), mounted_service_id.clone()],
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        let counts = |id: &str| {
            rows.iter()
                .find(|row| row.service_id == id)
                .unwrap()
                .known_operations
                .as_ref()
        };
        assert_eq!(
            counts(&catalog_service_id).map(|counts| counts.read),
            Some(1)
        );
        assert!(counts(&mounted_service_id).is_none());
    }

    #[tokio::test]
    async fn consent_disclosure_projects_visible_connections_and_published_metadata() {
        let Some(db) = connect_test_database("oauth_consent_disclosure").await else {
            return;
        };
        let id = || Uuid::new_v4().to_string();
        let actor = id();
        let another_user = id();
        let org = id();
        let viewer_org = id();
        let catalog_id = id();
        let provider = id();
        let oauth_key = id();
        let api_key = id();
        let oauth_id = id();
        let api_id = id();
        let disabled_id = id();
        let foreign_id = id();
        let org_hidden_id = id();
        let viewer_id = id();
        let corrupt_link = id();
        db.collection::<Document>(org_membership::COLLECTION_NAME)
            .insert_many([
                doc! {
                    "_id": id(), "member_user_id": &actor, "org_user_id": &org,
                    "role": "member", "allowed_service_ids": [&api_id], "created_at": DateTime::now(),
                },
                doc! {
                    "_id": id(), "member_user_id": &actor, "org_user_id": &viewer_org,
                    "role": "viewer", "created_at": DateTime::now(),
                },
            ])
            .await.unwrap();
        db.collection::<Document>(user_service::COLLECTION_NAME)
            .insert_many([
                (&oauth_id, &actor, &oauth_key, true, Some(&catalog_id)),
                (&api_id, &org, &api_key, true, None),
                (&disabled_id, &actor, &oauth_key, false, Some(&catalog_id)),
                (&foreign_id, &another_user, &api_key, true, None),
                (&org_hidden_id, &org, &api_key, true, None),
                (&viewer_id, &viewer_org, &api_key, true, None),
                (&corrupt_link, &actor, &api_key, true, None),
            ].into_iter().map(|(service, owner, key, active, catalog)| doc! {
                "_id": service, "user_id": owner, "api_key_id": key,
                "slug": service, "endpoint_id": id(), "is_active": active,
                "auth_method": "bearer", "auth_key_name": "Authorization",
                "catalog_service_id": catalog, "created_at": DateTime::now(), "updated_at": DateTime::now(),
            }))
            .await.unwrap();
        db.collection::<Document>(user_api_key::COLLECTION_NAME)
            .insert_many([
                doc! {
                    "_id": &oauth_key, "user_id": &actor, "status": "active", "credential_type": "oauth2",
                    "provider_config_id": &provider, "token_scopes": "repo provider:custom",
                    "access_token_encrypted": "never returned",
                },
                doc! {
                    "_id": &api_key, "user_id": &org, "status": "active", "credential_type": "api_key",
                    "provider_config_id": &provider, "token_scopes": "must not be treated as OAuth",
                },
            ])
            .await.unwrap();
        db.collection::<Document>(provider_config::COLLECTION_NAME)
            .insert_one(doc! { "_id": provider, "slug": "github" })
            .await
            .unwrap();
        db.collection::<Document>(downstream_service::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &catalog_id, "is_active": true,
                "inference": { "wire_protocol": "openai_responses" },
                "capabilities": { "supports_proxy_binary_upload": true },
            })
            .await
            .unwrap();
        db.collection::<Document>(service_endpoint::COLLECTION_NAME)
            .insert_many([
                ("GET", Some("read"), Some("published"), true),
                ("POST", Some("write"), Some("published"), true),
                ("DELETE", Some("write"), Some("published"), true),
                ("POST", None, None, true),
                ("GET", Some("read"), Some("draft"), true),
                ("POST", Some("write"), Some("paused"), true),
                ("DELETE", Some("write"), Some("published"), false),
            ].into_iter().map(|(method, risk, publication, active)| {
                let mut row = doc! {
                    "_id": id(), "service_id": &catalog_id, "method": method, "is_active": active,
                };
                if let Some(risk) = risk { row.insert("risk", risk); }
                if let Some(state) = publication { row.insert("publication", state); }
                row
            })).await.unwrap();

        let ids = vec![
            oauth_id.clone(),
            api_id.clone(),
            disabled_id,
            foreign_id,
            org_hidden_id,
            viewer_id,
            corrupt_link.clone(),
            oauth_id.clone(),
            id(),
        ];
        let rows = list(&db, &actor, &ids).await.unwrap();
        assert_eq!(rows.len(), 3);
        let row = |service_id: &str| {
            rows.iter()
                .find(|row| row.service_id == service_id)
                .unwrap()
        };
        let oauth = row(&oauth_id);
        assert!(matches!(oauth.source, CredentialSource::Personal));
        assert_eq!(oauth.credential_kind, CredentialKind::OAuth);
        assert_eq!(oauth.upstream_scopes.as_ref().unwrap().len(), 2);
        assert_eq!(
            oauth.known_operations,
            Some(OperationCounts {
                read: 1,
                write: 1,
                delete: 1,
                unclassified: 1
            })
        );
        assert_eq!(oauth.model_calls, Some(true));
        assert_eq!(oauth.binary_upload, Some(true));

        let api = row(&api_id);
        assert!(matches!(
            api.source,
            CredentialSource::Org { allowed: true, .. }
        ));
        assert_eq!(api.credential_kind, CredentialKind::ApiKey);
        assert!(api.upstream_scopes.is_none());
        assert!(api.known_operations.is_none());
        assert_eq!(api.model_calls, None);
        assert_eq!(api.binary_upload, None);

        let unknown = row(&corrupt_link);
        assert_eq!(unknown.credential_kind, CredentialKind::Unknown);
        assert!(unknown.upstream_scopes.is_none());
    }
}
