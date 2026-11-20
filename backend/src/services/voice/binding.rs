//! Explicit owner publication. Binding and service IDs are independent of names.
//! Execution grants remain ordinary UserService grants; publication grants no Key.
use super::access::{BOUND_SOURCE, SERVICE_SLUG};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent, downstream_service::DownstreamService,
        ssh_auth_mode::SshAuthMode, user_endpoint::UserEndpoint, user_service::UserService,
    },
    services::{assistant_team_service as team, service_history},
};
use mongodb::{
    Database,
    bson::{self, doc},
};

pub async fn get(db: &Database, owner: &str, agent: &str) -> AppResult<Option<UserService>> {
    db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .find_one(
            doc! {"user_id":owner,"source":BOUND_SOURCE,"source_id":agent,
            "deleted_at":bson::Bson::Null},
        )
        .await
        .map_err(Into::into)
}

pub struct Publication {
    pub agent: AssistantAgent,
    pub service: Option<UserService>,
    pub can_publish: bool,
}

/// Project the same live publication authority for the UI and the write path.
/// A visible Bot does not grant access to its voice connection metadata.
pub async fn publication(db: &Database, actor: &str, agent_id: &str) -> AppResult<Publication> {
    super::access::active_owner(db, actor).await?;
    let agent = team::agent(db, actor, agent_id).await?;
    let access = crate::services::org_agent_service::access(db, actor, &agent.user_id).await?;
    let mut service = None;
    let can_publish = if agent.destroyed_at.is_none() && access.can_write() {
        service = get(db, &agent.user_id, agent_id).await?;
        service.as_ref().map_or_else(
            || access.allows_any_resource(&[]),
            |service| access.allows_resource(&service.id),
        )
    } else {
        false
    };
    Ok(Publication {
        agent,
        service: service.filter(|_| can_publish),
        can_publish,
    })
}

pub async fn managed_agent(
    db: &Database,
    actor: &str,
    agent_id: &str,
) -> AppResult<AssistantAgent> {
    let publication = publication(db, actor, agent_id).await?;
    if !publication.can_publish {
        return Err(AppError::Forbidden(
            "Bot voice publication requires owner or organization service administration access"
                .into(),
        ));
    }
    Ok(publication.agent)
}

pub async fn publish(
    db: &Database,
    actor: &str,
    agent_id: &str,
    base_url: &str,
) -> AppResult<UserService> {
    let agent = managed_agent(db, actor, agent_id).await?;
    let catalog = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"slug":SERVICE_SLUG,"is_active":true})
        .await?
        .ok_or_else(|| {
            AppError::NotFound("NyxBot Voice has not been published in the service catalog".into())
        })?;
    let db = db.clone();
    let owner = agent.user_id.clone();
    let actor = actor.to_owned();
    let base_url = base_url.to_owned();
    service_history::transaction::run(&db.clone(), async move |tx| {
        let result: AppResult<UserService> = async {
            if actor != owner {
                crate::services::org_membership_authority::fence_live_member(&db, &actor, tx)
                    .await?;
                crate::services::org_membership_authority::fence_org(&db, &owner, tx).await?;
                managed_agent(&db, &actor, &agent.id).await?;
            }
            // Serialize concurrent publication and destruction on the existing Bot row.
            if db
                .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&agent.id,"user_id":&owner,"destroyed_at":bson::Bson::Null},
                    doc! {"$inc":{"voice_service_generation":1_i64}},
                )
                .session(&mut **tx)
                .await?
                .matched_count
                != 1
            {
                return Err(AppError::Forbidden("Bot is no longer available".into()));
            }
            let existing = db
                .collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
                .find_one(
                    doc! {"user_id":&owner,"source":BOUND_SOURCE,"source_id":&agent.id,
                    "catalog_service_id":&catalog.id,"deleted_at":bson::Bson::Null},
                )
                .session(&mut **tx)
                .await?;
            let id = uuid::Uuid::new_v4();
            if actor != owner {
                let new_service_id = id.to_string();
                crate::services::api_key_scope_service::validate_owner_service_write_with_session(
                    &db,
                    &owner,
                    existing
                        .as_ref()
                        .map(|service| service.id.as_str())
                        .unwrap_or(&new_service_id),
                    crate::services::api_key_scope_service::ScopeAuthorization::ActorPermissions {
                        actor_user_id: &actor,
                    },
                    tx,
                )
                .await?;
            }
            if let Some(existing) = existing {
                return Ok(existing);
            }
            let now = chrono::Utc::now();
            let endpoint = UserEndpoint {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: owner.clone(),
                label: format!(
                    "{} · Voice",
                    agent.display_name.as_deref().unwrap_or(&agent.name)
                ),
                url: super::execution::service_url(&base_url, &id.to_string())?,
                catalog_service_id: Some(catalog.id.clone()),
                // Voice is an HTTP/WebSocket service, not an MCP tool catalog.
                openapi_spec_url: None,
                recommended_skills: None,
                created_at: now,
                updated_at: now,
            };
            let service = UserService {
                id: id.to_string(),
                user_id: owner.clone(),
                slug: format!("nyxbot-voice-{}", id.simple()),
                deleted_at: None,
                created_by: None,
                last_change: None,
                endpoint_id: endpoint.id.clone(),
                api_key_id: None,
                credential_binding: None,
                catalog_service_id: Some(catalog.id.clone()),
                icon_url: None,
                node_id: None,
                node_priority: 0,
                service_type: "http".into(),
                ssh_auth_mode: SshAuthMode::ProxyOnly,
                admin_only: false,
                ssh_node_keys_stale: false,
                auth_method: "none".into(),
                auth_key_name: String::new(),
                identity_propagation_mode: "none".into(),
                identity_include_user_id: false,
                identity_include_email: false,
                identity_include_name: false,
                identity_jwt_audience: None,
                forward_access_token: false,
                inject_delegation_token: false,
                delegation_token_scope: "llm:proxy".into(),
                custom_user_agent: None,
                default_request_headers: None,
                ws_frame_injections: vec![],
                is_active: true,
                source: Some(BOUND_SOURCE.into()),
                source_id: Some(agent.id.clone()),
                source_app_id: None,
                created_at: now,
                updated_at: now,
                state_version: 1,
                rotation_predecessor_id: None,
                credential_verification: None,
            };
            service_history::collection::<UserEndpoint>(
                &db,
                crate::models::user_endpoint::COLLECTION_NAME,
            )
            .insert_one(endpoint)
            .session(&mut *tx)
            .await?;
            service_history::collection::<UserService>(
                &db,
                crate::models::user_service::COLLECTION_NAME,
            )
            .insert_one(&service)
            .session(&mut *tx)
            .await?;
            Ok(service)
        }
        .await;
        crate::services::api_key_mutation_service::transaction_result(result)
    })
    .await
    .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}
