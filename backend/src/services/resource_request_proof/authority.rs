use super::*;
use crate::models::{
    api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS},
    api_key_credential::{ApiKeyCredential, COLLECTION_NAME as CREDENTIALS},
    user_service::{AUTO_PROVISION_SOURCE, COLLECTION_NAME as CONNECTIONS, UserService},
};
use mongodb::bson::{Document, doc};
use sha2::{Digest, Sha256};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct AuthorityBinding {
    pub key_id: String,
    pub credential_id: Option<String>,
    pub credential_incarnation: String,
    pub connection_id: String,
    pub catalog_id: String,
    pub connection_binding: String,
    pub origin_binding: String,
    pub approval_service_id: String,
    pub proxy_path: String,
    pub method: String,
}

pub(super) async fn catalog(db: &Database, id: &str) -> AppResult<DownstreamService> {
    let service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": id, "is_active": true})
        .await?
        .ok_or_else(refused)?;
    if service.service_type != "http"
        || !matches!(service.identity_propagation_mode.as_str(), "jwt" | "both")
        || service
            .introspection_client_ids
            .as_ref()
            .is_none_or(Vec::is_empty)
        || !super::super::platform_key_service::no_auth_platform_service(&service)
        || service.requires_user_credential
        || service.offering_kind == crate::models::downstream_service::OfferingKind::Tool
        || super::super::retired_service_service::is_retired(&service)
        || db
            .collection::<Document>(crate::models::service_provider_requirement::COLLECTION_NAME)
            .find_one(doc! {"service_id": id})
            .await?
            .is_some()
    {
        return Err(refused());
    }
    Ok(service)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn load(
    db: &Database,
    subject: &str,
    key_id: &str,
    credential_id: Option<&str>,
    connection_id: &str,
    catalog_id: &str,
    approval_service_id: &str,
    method: &str,
    proxy_path: &str,
) -> AppResult<AuthorityBinding> {
    // Catalog-backed HTTP and MCP admission select the catalog policy. Never
    // let a connection alias change which policy is rechecked at consumption.
    if approval_service_id != catalog_id {
        return Err(refused());
    }
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id": key_id, "user_id": subject, "is_active": true})
        .await?
        .ok_or_else(refused)?;
    if key.purpose != ApiKeyPurpose::General
        || key.expires_at.is_some_and(|at| at <= Utc::now())
        || !key.assistant_operation_scopes.is_empty()
        || key.assistant_agent_owner_id.is_some()
        || key.assistant_group_id.is_some()
    {
        return Err(refused());
    }
    let child = if let Some(id) = credential_id {
        let child = db
            .collection::<ApiKeyCredential>(CREDENTIALS)
            .find_one(doc! {"_id": id, "api_key_id": key_id, "user_id": subject, "is_active": true})
            .await?
            .ok_or_else(refused)?;
        if child.expires_at.is_some_and(|at| at <= Utc::now()) {
            return Err(refused());
        }
        Some(child)
    } else {
        None
    };
    let auth =
        crate::mw::auth::api_key_auth_user(db, &key, credential_id.map(str::to_owned), None, None)
            .await?;
    auth.ensure_rest_proxy_access()?;
    let chat = auth.assistant_chat.as_ref().ok_or_else(refused)?;
    // V1 does not translate specialist/guest authority into owner authority.
    // It is the exact owner's already-admitted NyxBot read only.
    if !chat.is_orchestrator()
        || chat.guest
        || !chat.turn_live
        || chat.turn_stopped
        || chat.turn_id.is_none()
        || auth.org_agent_access.is_some()
    {
        return Err(refused());
    }
    auth.ensure_live_assistant_turn(db, "resource_request.introspection")
        .await?;
    if !auth.allow_all_services
        && !auth
            .allowed_service_ids
            .iter()
            .any(|id| id == connection_id || id == catalog_id)
    {
        return Err(refused());
    }
    let connection = db.collection::<UserService>(CONNECTIONS)
        .find_one(doc! {"_id": connection_id, "user_id": subject, "catalog_service_id": catalog_id, "is_active": true, "deleted_at": null})
        .await?.ok_or_else(refused)?;
    if connection.node_id.is_some()
        || connection.service_type != "http"
        || connection.auth_method != "none"
        || connection.api_key_id.is_some()
    {
        return Err(refused());
    }
    let service = catalog(db, catalog_id).await?;
    super::super::proxy_service::verify_auto_provision_eligibility(db, &connection, subject)
        .await?;
    if service.platform_key.is_some() {
        super::super::platform_key_service::require(db, &service, subject).await?;
    }
    let operation =
        super::super::operation_descriptor::build_http_descriptor(method, proxy_path, None);
    let approval = super::super::approval_service::resolve_org_aware_approval(
        db,
        subject,
        subject,
        approval_service_id,
        &operation,
        connection.source.as_deref() == Some(AUTO_PROVISION_SOURCE),
    )
    .await?;
    if approval.required
        || approval.effect != crate::models::service_approval_config::ApprovalEffect::AutoAllow
    {
        return Err(refused());
    }
    let origin = db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": &chat.conversation_id, "user_id": subject})
        .projection(doc! {
            "agent_id":1, "agent_owner_id":1, "group_id":1, "guest_turn":1,
            "credential_api_key_id":1, "invocation_controlled":1, "credential_invocation_id":1,
            "platform_thread":1, "active_turn.turn_id":1, "active_turn.stop_requested":1,
            "active_turn.invocation_id":1, "active_turn.platform_origin":1,
            "active_turn.channel_adapter_origin":1, "active_turn.trigger_run_id":1,
        })
        .await?
        .ok_or_else(refused)?;
    let incarnation = serde_json::to_vec(&(
        key_id,
        &key.key_hash,
        child.as_ref().map(|child| (&child.id, &child.secret_hash)),
    ))
    .map_err(|_| refused())?;
    Ok(AuthorityBinding {
        key_id: key_id.into(),
        credential_id: credential_id.map(str::to_owned),
        credential_incarnation: format!("{:x}", Sha256::digest(incarnation)),
        connection_id: connection_id.into(),
        catalog_id: catalog_id.into(),
        connection_binding: super::super::assistant_service_execution::connection_binding(
            db,
            connection_id,
            key_id,
        )
        .await?,
        origin_binding: super::super::mcp_service::canonical_sha256(
            serde_json::to_value(origin).map_err(|_| refused())?,
        ),
        approval_service_id: approval_service_id.into(),
        proxy_path: proxy_path.into(),
        method: method.into(),
    })
}

impl AuthorityBinding {
    pub(super) async fn require_current(&self, db: &Database, subject: &str) -> AppResult<()> {
        let fresh = load(
            db,
            subject,
            &self.key_id,
            self.credential_id.as_deref(),
            &self.connection_id,
            &self.catalog_id,
            &self.approval_service_id,
            &self.method,
            &self.proxy_path,
        )
        .await?;
        if &fresh != self {
            return Err(refused());
        }
        Ok(())
    }
}
