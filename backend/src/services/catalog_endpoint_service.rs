//! Refresh catalog-owned transport snapshots without recreating connections.
use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService},
        provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig},
        user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint},
        user_service::{AUTO_PROVISION_SOURCE, COLLECTION_NAME as CONNECTIONS, UserService},
    },
};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use std::{collections::HashMap, time::Duration};

const PAGE_SIZE: i64 = 100;

pub(crate) fn owned(connection: &UserService) -> bool {
    connection.source.as_deref() == Some(AUTO_PROVISION_SOURCE)
        && connection.deleted_at.is_none()
        && connection.service_type == "http"
        && connection.node_id.is_none()
        && connection.api_key_id.is_none()
        && connection.catalog_service_id.is_some()
}

fn linked(connection: &UserService, endpoint: &UserEndpoint, catalog: &DownstreamService) -> bool {
    owned(connection)
        && catalog.service_type == "http"
        && endpoint.id == connection.endpoint_id
        && endpoint.user_id == connection.user_id
        && endpoint.catalog_service_id.as_deref() == Some(catalog.id.as_str())
        && connection.catalog_service_id.as_deref() == Some(catalog.id.as_str())
}

/// Gateway-URL providers own a per-account destination, never a catalog target.
pub(crate) async fn catalog_owns_url(
    db: &Database,
    catalog: &DownstreamService,
) -> AppResult<bool> {
    let Some(id) = &catalog.provider_config_id else {
        return Ok(catalog.service_type == "http");
    };
    Ok(db
        .collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! {"_id":id})
        .await?
        .is_some_and(|provider| !provider.requires_gateway_url && catalog.service_type == "http"))
}

pub(crate) fn apply_live_target(
    connection: &UserService,
    endpoint: &mut UserEndpoint,
    catalog_id: &str,
    url: &str,
    spec: &Option<String>,
) {
    if owned(connection)
        && endpoint.user_id == connection.user_id
        && endpoint.id == connection.endpoint_id
        && endpoint.catalog_service_id.as_deref() == Some(catalog_id)
        && connection.catalog_service_id.as_deref() == Some(catalog_id)
    {
        endpoint.url = url.to_owned();
        endpoint.openapi_spec_url.clone_from(spec);
    }
}

/// IDs only enter this transaction; every retry reads current authority/targets.
async fn repair(db: &Database, id: &str) -> AppResult<bool> {
    super::transaction_service::run_db_only(db, &(db.clone(), id.to_owned()), |tx, (db, id)| {
        Box::pin(repair_in_session(db, tx, id))
    })
    .await
}

async fn repair_in_session(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    id: &str,
) -> AppResult<bool> {
    let Some(connection) = db
        .collection::<UserService>(CONNECTIONS)
        .find_one(doc! {"_id":id})
        .session(&mut *tx)
        .await?
    else {
        return Ok(false);
    };
    if !owned(&connection) {
        return Ok(false);
    }
    let Some(catalog) = db
        .collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"_id":&connection.catalog_service_id})
        .session(&mut *tx)
        .await?
    else {
        return Ok(false);
    };
    let Some(endpoint) = db
        .collection::<UserEndpoint>(ENDPOINTS)
        .find_one(doc! {"_id":&connection.endpoint_id,"deleted_at":null})
        .session(&mut *tx)
        .await?
    else {
        return Ok(false);
    };
    if !linked(&connection, &endpoint, &catalog)
        || (endpoint.url == catalog.base_url
            && endpoint.openapi_spec_url == catalog.openapi_spec_url)
    {
        return Ok(false);
    }
    if let Some(provider_id) = &catalog.provider_config_id {
        let provider = db
            .collection::<ProviderConfig>(PROVIDERS)
            .find_one(doc! {"_id":provider_id})
            .session(&mut *tx)
            .await?;
        if provider.is_none_or(|p| p.requires_gateway_url) {
            return Ok(false);
        }
        // Serialize with provider edits too; this field is operational only.
        db.collection::<Document>(PROVIDERS)
            .update_one(
                doc! {"_id":provider_id},
                doc! {"$inc":{"authority_fence":1_i64}},
            )
            .session(&mut *tx)
            .await?;
    }
    // Never rewrite a snapshot shared with an explicitly configured connection.
    if db
        .collection::<Document>(CONNECTIONS)
        .find_one(doc! {"endpoint_id":&endpoint.id,"_id":{"$ne":id}})
        .session(&mut *tx)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    super::catalog_mutation_service::fence(db, tx, &catalog, None).await?;
    // Fence deletion/routing edits without touching connection revision or credentials.
    db.collection::<Document>(CONNECTIONS)
        .update_one(doc! {"_id":id}, doc! {"$inc":{"authority_fence":1_i64}})
        .session(&mut *tx)
        .await?;
    db.collection::<Document>(ENDPOINTS)
        .update_one(
            doc! {"_id":&endpoint.id},
            doc! {"$set":{
                "url":&catalog.base_url,"openapi_spec_url":&catalog.openapi_spec_url,
                "updated_at":bson::DateTime::now()
            }},
        )
        .session(&mut *tx)
        .await?;
    let key = super::audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit service unavailable".into()))?;
    super::audit_service::log_system_event_in_session(db, tx, key,
        "catalog_endpoint_reconciled", serde_json::json!({
            "catalog_service_id":catalog.id,"user_service_id":connection.id,"endpoint_id":endpoint.id,
            "target_changed":endpoint.url != catalog.base_url,
            "spec_changed":endpoint.openapi_spec_url != catalog.openapi_spec_url
        })).await?;
    Ok(true)
}

/// Advisory snapshots only: two batched reads select mismatches. Repair still
/// reloads all authority and fences concurrent changes inside its transaction.
async fn mismatched_ids(db: &Database, connections: &[UserService]) -> AppResult<Vec<String>> {
    let owned: Vec<_> = connections.iter().filter(|row| self::owned(row)).collect();
    if owned.is_empty() {
        return Ok(Vec::new());
    }
    let endpoint_ids: Vec<_> = owned.iter().map(|row| row.endpoint_id.as_str()).collect();
    let catalog_ids: Vec<_> = owned
        .iter()
        .filter_map(|row| row.catalog_service_id.as_deref())
        .collect();
    let endpoints: HashMap<String, UserEndpoint> = db
        .collection::<UserEndpoint>(ENDPOINTS)
        .find(doc! {"_id":{"$in":endpoint_ids},"deleted_at":null})
        .await?
        .map_ok(|row| (row.id.clone(), row))
        .try_collect()
        .await?;
    let catalogs: HashMap<String, DownstreamService> = db
        .collection::<DownstreamService>(CATALOG)
        .find(doc! {"_id":{"$in":catalog_ids}})
        .await?
        .map_ok(|row| (row.id.clone(), row))
        .try_collect()
        .await?;
    Ok(owned
        .into_iter()
        .filter_map(|row| {
            let endpoint = endpoints.get(&row.endpoint_id)?;
            let catalog = catalogs.get(row.catalog_service_id.as_deref()?)?;
            (linked(row, endpoint, catalog)
                && (endpoint.url != catalog.base_url
                    || endpoint.openapi_spec_url != catalog.openapi_spec_url))
                .then(|| row.id.clone())
        })
        .collect())
}

/// Bounded-memory keyset walk. Each page uses three find queries; consistent
/// rows never open transactions or write. One look-ahead row avoids an extra
/// empty query when the final page contains exactly PAGE_SIZE connections.
pub(crate) async fn reconcile(
    db: &Database,
    owner: Option<&str>,
    catalog: Option<&str>,
) -> AppResult<u64> {
    let mut after = String::new();
    let mut changed = 0;
    loop {
        let mut filter = doc! {"source":AUTO_PROVISION_SOURCE,"deleted_at":null,"node_id":null,"api_key_id":null,"service_type":{"$in":["http",null]},"_id":{"$gt":&after}};
        if let Some(owner) = owner {
            filter.insert("user_id", owner);
        }
        if let Some(catalog) = catalog {
            filter.insert("catalog_service_id", catalog);
        }
        let mut rows: Vec<UserService> = db
            .collection::<UserService>(CONNECTIONS)
            .find(filter)
            .sort(doc! {"_id":1})
            .limit(PAGE_SIZE + 1)
            .await?
            .try_collect()
            .await?;
        let has_more = rows.len() > PAGE_SIZE as usize;
        rows.truncate(PAGE_SIZE as usize);
        let Some(last) = rows.last() else {
            return Ok(changed);
        };
        for id in mismatched_ids(db, &rows).await? {
            if repair(db, &id).await? {
                changed += 1;
            }
        }
        if !has_more {
            return Ok(changed);
        }
        after.clone_from(&last.id);
        tokio::task::yield_now().await;
    }
}

/// Repair is maintenance, never a prerequisite for listing, login or approval.
/// Live target resolution remains authoritative when a stored repair fails.
pub(crate) async fn reconcile_owner(db: &Database, owner: &str) {
    if reconcile(db, Some(owner), None).await.is_err() {
        tracing::warn!(
            code = "catalog_endpoint_reconciliation_failed",
            scope = "owner",
            "Catalog endpoint reconciliation failed; continuing provisioning"
        );
    }
}

#[cfg(test)]
tokio::task_local! {
    static BACKGROUND_PAUSE: (std::sync::Arc<tokio::sync::Notify>, std::sync::Arc<tokio::sync::Notify>);
}

/// Admin responses and startup never await the platform-wide walk. Pages and
/// transactions stay bounded; a five-minute pass limit also bounds each task.
pub(crate) fn spawn_reconcile(db: Database, catalog: Option<String>) {
    #[cfg(test)]
    let pause = BACKGROUND_PAUSE.try_with(Clone::clone).ok();
    tokio::spawn(async move {
        #[cfg(test)]
        if let Some((entered, release)) = pause {
            entered.notify_one();
            release.notified().await;
        }
        let scope = if catalog.is_some() {
            "catalog"
        } else {
            "startup"
        };
        match tokio::time::timeout(
            Duration::from_secs(300),
            reconcile(&db, None, catalog.as_deref()),
        )
        .await
        {
            Ok(Ok(changed)) => {
                tracing::debug!(scope, changed, "Catalog endpoint reconciliation complete")
            }
            Ok(Err(_)) => tracing::warn!(
                scope,
                code = "catalog_endpoint_reconciliation_failed",
                "Catalog endpoint reconciliation failed"
            ),
            Err(_) => tracing::warn!(
                scope,
                code = "catalog_endpoint_reconciliation_timed_out",
                "Catalog endpoint reconciliation reached its time limit"
            ),
        }
    });
}

#[cfg(test)]
mod tests;
