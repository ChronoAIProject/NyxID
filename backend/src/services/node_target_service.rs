//! Metadata-only synchronization for endpoints whose secret exists only on a node.
use std::sync::Arc;

use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Document, doc},
    options::ReturnDocument,
};
use serde::Serialize;
use utoipa::ToSchema;

use super::{
    node_ws_manager::{CredentialUpdateParams, NodeWsManager},
    user_endpoint_service,
};
use crate::{
    errors::{AppError, AppResult},
    models::{user_api_key::UserApiKey, user_service::UserService},
};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TargetSync {
    pub service_id: String,
    pub target_revision: i64,
    /// synced, offline, unsupported, or unconfirmed. The server URL is committed.
    pub status: String,
}

pub struct Migration {
    services: Vec<UserService>,
    old_url: String,
    new_url: String,
    owner: String,
    endpoint: String,
    revision: Option<i64>,
}

impl Migration {
    /// Check every affected node-local service, including shared endpoint rows.
    pub async fn prepare(
        db: &Database,
        actor: &str,
        owner: &str,
        endpoint: &str,
        new_url: &str,
    ) -> AppResult<Self> {
        user_endpoint_service::validate_endpoint_url(new_url)?;
        let mut old_url = user_endpoint_service::get_endpoint(db, owner, endpoint)
            .await?
            .url;
        let services: Vec<UserService> =
            super::service_history::collection(db, crate::models::user_service::COLLECTION_NAME)
                .find(doc! {"user_id": owner, "endpoint_id": endpoint})
                .await?
                .try_collect()
                .await?;
        let access = super::org_service::resolve_owner_access(db, actor, owner).await?;
        let mut targets = Vec::new();
        for service in services {
            let Some(key) = service.api_key_id.as_ref() else {
                continue;
            };
            let key: Option<UserApiKey> = super::service_history::collection(
                db,
                crate::models::user_api_key::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": key, "user_id": owner})
            .await?;
            if !key.is_some_and(|key| key.credential_type == "node_managed") {
                continue;
            }
            if !access.can_write() || !access.allows_resource(&service.id) {
                return Err(AppError::Forbidden(
                    "Write access to every service sharing this endpoint is required".into(),
                ));
            }
            if let Some(node) = service.node_id.as_deref().filter(|node| !node.is_empty()) {
                super::node_service::ensure_node_writable_by_actor(db, actor, node).await?;
            }
            targets.push(service);
        }
        // Reserve an ordered metadata version before any node I/O. Keeping it
        // on the endpoint lets the subsequent URL write use one atomic CAS.
        // Older rows have no version; ordinary readers ignore this metadata.
        let revision = if targets.is_empty() {
            None
        } else {
            let row = super::service_history::collection::<Document>(
                db,
                crate::models::user_endpoint::COLLECTION_NAME,
            )
            .find_one_and_update(
                doc! {"_id": endpoint, "user_id": owner},
                doc! {"$inc": {"node_target_revision": 1_i64}},
            )
            .return_document(ReturnDocument::After)
            .await?
            .ok_or_else(|| AppError::NotFound("Endpoint not found".into()))?;
            old_url = row
                .get_str("url")
                .map_err(|_| AppError::Internal("Invalid endpoint URL".into()))?
                .to_string();
            Some(
                row.get_i64("node_target_revision")
                    .map_err(|_| AppError::Internal("Invalid endpoint target revision".into()))?,
            )
        };
        Ok(Self {
            owner: owner.into(),
            endpoint: endpoint.into(),
            revision,
            services: targets,
            old_url,
            new_url: new_url.to_string(),
        })
    }

    async fn sync(&self, manager: &Arc<NodeWsManager>, service: &UserService) -> TargetSync {
        let Some(node) = service.node_id.as_deref().filter(|node| !node.is_empty()) else {
            // Legacy detached keys may still route through node bindings. A
            // nonempty server URL remains authoritative, but do not enable an
            // unconfirmed local fallback when there is no single sync target.
            return TargetSync {
                service_id: service.id.clone(),
                target_revision: self.revision.expect("prepared revision"),
                status: "unsupported".into(),
            };
        };
        let info = manager.cluster_session_info(node).await;
        let status = if !info.is_connected {
            "offline"
        } else if !info.capabilities.credential_target_update_v1
            || !info.capabilities.credential_ack_correlation
        {
            "unsupported"
        } else {
            let params = CredentialUpdateParams {
                service_slug: service.slug.clone(),
                injection_method: "target_url".into(),
                target_url: Some(self.new_url.clone()),
                target_revision: self.revision,
                header_name: None,
                header_value: None,
                param_name: None,
                param_value: None,
            };
            match manager
                .send_credential_update_and_wait_clustered(
                    node,
                    &params,
                    std::time::Duration::from_secs(3),
                )
                .await
            {
                Ok(()) => "synced",
                Err(_) => "unconfirmed", // Never propagate node-controlled messages or URL material.
            }
        };
        TargetSync {
            service_id: service.id.clone(),
            target_revision: self.revision.expect("prepared revision"),
            status: status.into(),
        }
    }

    /// An empty server URL enables local fallback. Never commit it while that
    /// fallback may still contain the old destination. Legacy/offline nodes
    /// must upgrade/reconnect first; nonempty migrations do not need this gate.
    pub async fn before_commit(&self, manager: &Arc<NodeWsManager>) -> AppResult<()> {
        if self.new_url.is_empty() {
            for service in &self.services {
                let result = self.sync(manager, service).await;
                if result.status != "synced" {
                    return Err(AppError::BadRequest(format!(
                        "Cannot clear a node-managed endpoint until its local target is cleared ({}); reconnect or upgrade the node, or set a nonempty endpoint_url",
                        result.status
                    )));
                }
            }
        }
        Ok(())
    }

    /// Fence the URL write against newer migrations on either edit route.
    pub async fn commit(
        &self,
        db: &Database,
        label: Option<&str>,
        spec: user_endpoint_service::OpenApiSpecUrlUpdate<'_>,
        skills: user_endpoint_service::RecommendedSkillsUpdate,
    ) -> AppResult<()> {
        let update =
            user_endpoint_service::build_endpoint_update(Some(&self.new_url), label, spec, skills)?;
        let mut filter = doc! {"_id": &self.endpoint, "user_id": &self.owner};
        if let Some(revision) = self.revision {
            filter.insert("node_target_revision", revision);
        }
        let result = super::service_history::collection::<Document>(
            db,
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .update_one(filter, update)
        .await?;
        if result.matched_count == 0 {
            return Err(AppError::BadRequest(
                "Endpoint changed during migration; reload and retry".into(),
            ));
        }
        Ok(())
    }

    pub async fn after_commit(
        self,
        db: &Database,
        manager: &Arc<NodeWsManager>,
        actor: super::audit_service::AuditActor,
    ) -> Vec<TargetSync> {
        let mut results = Vec::new();
        for service in &self.services {
            let result = if self.new_url.is_empty() {
                TargetSync {
                    service_id: service.id.clone(),
                    target_revision: self.revision.expect("prepared revision"),
                    status: "synced".into(),
                }
            } else {
                self.sync(manager, service).await
            };
            super::audit_service::log_async(
                db.clone(),
                Some(actor.user_id.clone()),
                "node_managed_endpoint_updated".into(),
                Some(serde_json::json!({
                    "service_id": service.id, "old_host": host(&self.old_url), "new_host": host(&self.new_url), "node_target_sync": result.status,
                })),
                actor.ip_address.clone(),
                actor.user_agent.clone(),
                actor.api_key_id.clone(),
                actor.api_key_name.clone(),
            );
            results.push(result);
        }
        results
    }
}

fn host(raw: &str) -> Option<String> {
    url::Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}
