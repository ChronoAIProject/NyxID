//! Durable shared execution keeps immutable provenance, never an ACL snapshot.
use super::{shared_agent_consent as consent, shared_agent_service as sharing};
use crate::{
    errors::{AppError, AppResult},
    models::{api_key::ApiKey, assistant_sharing::SharedExecution},
};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};

pub const READINESS: &str = "assistant:shared-agent-transports-v1";

pub async fn require_ready(db: &Database) -> AppResult<()> {
    super::security_readiness_service::require(db, READINESS).await
}

/// Opaque frames cannot prove a method/path/verb policy. Only an unrestricted
/// service can admit a stream; an allowed GET handshake is not write authority.
pub async fn require_stream_policy(
    db: &Database,
    actor: &str,
    connection: &crate::models::user_service::UserService,
    service: &crate::models::downstream_service::DownstreamService,
) -> AppResult<()> {
    use crate::models::service_approval_config::{ApprovalEffect, ServiceApprovalConfig};
    let refused = || {
        AppError::Forbidden(
            "Shared streams require a service without operation or approval restrictions".into(),
        )
    };
    if service.proxy_operation_policy.is_some() {
        return Err(refused());
    }
    let configs: Vec<ServiceApprovalConfig> = db
        .collection(crate::models::service_approval_config::COLLECTION_NAME)
        .find(doc! {"user_id":{"$in":[actor, &connection.user_id]},
            "service_id":{"$in":[connection.id.as_str(), connection.catalog_service_id.as_deref().unwrap_or(&service.id)]}})
        .await?
        .try_collect()
        .await?;
    if configs.iter().any(|config| {
        config
            .rules
            .iter()
            .any(|rule| rule.effect != ApprovalEffect::AutoAllow)
            || config
                .default_effect
                .as_ref()
                .is_some_and(|effect| *effect != ApprovalEffect::AutoAllow)
            || config.default_effect.is_none()
                && config.rules.is_empty()
                && config.approval_required
    }) {
        return Err(refused());
    }
    // A global approval requirement applies to all later writes as well. Use
    // the ordinary resolver for implicit/auto-connected policy defaults.
    let descriptor = super::operation_descriptor::build_http_descriptor("DELETE", "/", None);
    for principal in [actor, connection.user_id.as_str()] {
        let policy = super::approval_service::resolve_org_aware_approval(
            db,
            principal,
            principal,
            &connection.id,
            &descriptor,
            connection.source.as_deref() == Some("auto_provision"),
        )
        .await?;
        if policy.required || policy.effect != ApprovalEffect::AutoAllow {
            return Err(refused());
        }
    }
    Ok(())
}

pub async fn revalidate(db: &Database, row: &SharedExecution) -> AppResult<()> {
    revalidate_inner(db, row, false).await
}

pub async fn revalidate_watch(
    db: &Database,
    watch: &crate::models::async_service_operation::AsyncServiceOperation,
) -> AppResult<()> {
    if let Some(binding) = &watch.shared_execution {
        revalidate_inner(db, binding, watch.state == "cancelling").await?;
    }
    Ok(())
}

async fn revalidate_inner(db: &Database, row: &SharedExecution, cleanup: bool) -> AppResult<()> {
    require_ready(db).await?;
    let key = db
        .collection::<ApiKey>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": &row.key, "user_id": &row.actor, "is_active": true})
        .await?
        .ok_or_else(sharing::missing)?;
    super::key_service::require_ordinary_purpose(key.purpose)?;
    if key.expires_at.is_some_and(|at| at <= chrono::Utc::now()) {
        return Err(sharing::missing());
    }
    let access = super::org_agent_service::resolve_credential_access(db, &key)
        .await?
        .ok_or_else(sharing::missing)?;
    super::org_group_service::validate_key(db, &key, Some(&access)).await?;
    let shared = access.shared().ok_or_else(sharing::missing)?;
    if !shared.matches(&row.actor, &row.owner, &row.agent)
        || shared.revision() != row.revision
        || consent::behavior(shared) != row.behavior
    {
        return Err(sharing::missing());
    }
    // Deletion, rotation, and moving a key to another conversation revoke resume.
    let thread = db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &row.conversation, "user_id": &row.actor,
        "agent_id": &row.agent, "credential_api_key_id": &row.key,
        "shared_agent_thread": true})
        .await?
        .ok_or_else(sharing::missing)?;
    // These durable records bind a private turn, not a collaboration request
    // and its audience. Reusing a slot for another request must never donate
    // authority to an old job/watch/stream. Group queues use their own binding.
    if thread
        .get("collaboration")
        .is_some_and(|value| value != &mongodb::bson::Bson::Null)
    {
        return Err(AppError::Forbidden(
            "This transport requires a private shared chat; collaboration request binding is unavailable".into(),
        ));
    }
    if !row.voice && !cleanup {
        let stopped = db.collection::<mongodb::bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id": &row.conversation, "active_turn.turn_id": &row.turn, "active_turn.stop_requested": true})
            .projection(doc! {"_id":1}).await?.is_some()
            || db.collection::<mongodb::bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
                .find_one(doc! {"conversation_id": &row.conversation, "user_id": &row.actor, "turn_id": &row.turn,
                    "role":"assistant", "status":"stopped"}).projection(doc! {"_id":1}).await?.is_some();
        if stopped {
            return Err(AppError::AssistantTurnRequired);
        }
    }
    let service = super::user_service_service::find_user_service_by_id(db, &row.connection)
        .await?
        .ok_or_else(sharing::missing)?;
    if !(shared.allows_connection(&service)
        || row.model && shared.allows_model_connection(&service))
    {
        return Err(sharing::missing());
    }
    // Platform audience membership can change independently of the agent
    // share or catalog configuration; it is live authority for each effect.
    if (service.credential_binding.as_deref() == Some("platform") || service.api_key_id.is_none())
        && let Some(id) = &service.catalog_service_id
    {
        let catalog = db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":id,"is_active":true})
            .await?
            .ok_or_else(sharing::missing)?;
        if super::platform_key_service::has_platform_key(&catalog) {
            super::platform_key_service::require(db, &catalog, &service.user_id).await?;
            if service.user_id != row.actor {
                super::platform_key_service::require(db, &catalog, &row.actor).await?;
            }
        }
    }
    if service.user_id == row.actor {
        consent::Permit::from_execution(row)
            .ok_or_else(sharing::missing)?
            .revalidate(db, shared, &row.actor)
            .await?;
    } else if row.consent_id.is_some() {
        return Err(sharing::missing());
    }
    if super::assistant_service_execution::connection_binding(db, &row.connection, &row.key).await?
        != row.binding
    {
        return Err(AppError::Forbidden(
            "Shared execution authority changed".into(),
        ));
    }
    Ok(())
}

/// Metadata-only shared attribution; never include the consent claim or key.
pub fn annotate(event: &mut serde_json::Value, binding: Option<&SharedExecution>) {
    if let Some(row) = binding {
        event["execution_mode"] = "shared".into();
        event["actor_user_id"] = row.actor.clone().into();
        event["billing_owner_id"] = row.actor.clone().into();
        event["execution_principal_id"] = row.actor.clone().into();
        let personal = row.consent_id.is_some();
        event["credential_source"] = if personal {
            "actor_personal"
        } else {
            "owner_shared"
        }
        .into();
        event["credential_owner_id"] = if personal { &row.actor } else { &row.owner }
            .clone()
            .into();
        event["agent_owner_id"] = row.owner.clone().into();
        event["agent_id"] = row.agent.clone().into();
        event["sharing_revision"] = row.revision.into();
        event["conversation_id"] = row.conversation.clone().into();
        event["turn_id"] = row.turn.clone().into();
        event["connection_id"] = row.connection.clone().into();
    }
}

pub fn annotate_chat(
    event: &mut serde_json::Value,
    chat: &super::assistant_acknowledgement_service::ChatAuthority,
) {
    if let Some(access) = chat.org_agent_access.as_ref().and_then(|a| a.shared()) {
        event["execution_mode"] = "shared".into();
        event["actor_user_id"] = chat.user_id.clone().into();
        event["billing_owner_id"] = chat.user_id.clone().into();
        event["agent_owner_id"] = access.owner().into();
        event["agent_id"] = chat.agent_id.clone().into();
        event["conversation_id"] = chat.conversation_id.clone().into();
        event["turn_id"] = serde_json::json!(chat.turn_id);
        event["sharing_revision"] = access.revision().into();
    }
}
