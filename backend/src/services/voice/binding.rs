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
            "is_active":true,"deleted_at":bson::Bson::Null},
        )
        .await
        .map_err(Into::into)
}

pub async fn publish(db: &Database, owner: &str, agent_id: &str) -> AppResult<UserService> {
    super::access::personal_owner(db, owner).await?;
    let agent = team::agent(db, owner, agent_id).await?;
    if agent.user_id != owner || agent.destroyed_at.is_some() {
        return Err(AppError::Forbidden(
            "Only an active personal Bot's owner can publish voice access".into(),
        ));
    }
    let catalog = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"slug":SERVICE_SLUG,"is_active":true})
        .await?
        .ok_or_else(|| {
            AppError::NotFound("NyxBot Voice has not been published in the service catalog".into())
        })?;
    let db = db.clone();
    let owner = owner.to_owned();
    service_history::transaction::run(&db.clone(), async move |tx| {
        let result: AppResult<UserService> = async {
            // Serialize concurrent publication and destruction on the existing Bot row.
            if db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                .update_one(doc! {"_id":&agent.id,"user_id":&owner,"destroyed_at":bson::Bson::Null},
                    doc! {"$inc":{"voice_service_generation":1_i64}})
                .session(&mut **tx).await?.matched_count != 1 {
                return Err(AppError::Forbidden("Bot is no longer available".into()));
            }
            if let Some(existing) = db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
                .find_one(doc! {"user_id":&owner,"source":BOUND_SOURCE,"source_id":&agent.id,
                    "catalog_service_id":&catalog.id,"is_active":true,"deleted_at":bson::Bson::Null})
                .session(&mut **tx).await? {
                return Ok(existing);
            }
            let now = chrono::Utc::now();
            let endpoint = UserEndpoint {
                id: uuid::Uuid::new_v4().to_string(), user_id: owner.clone(),
                label: format!("{} · Voice", agent.display_name.as_deref().unwrap_or(&agent.name)),
                url: catalog.base_url.clone(), catalog_service_id: Some(catalog.id.clone()),
                openapi_spec_url: catalog.openapi_spec_url.clone(), recommended_skills: None,
                created_at: now, updated_at: now,
            };
            crate::services::user_endpoint_service::validate_endpoint_url(&endpoint.url)?;
            let id = uuid::Uuid::new_v4();
            let service = UserService {
                id: id.to_string(), user_id: owner.clone(), slug: format!("nyxbot-voice-{}", id.simple()),
                deleted_at: None, created_by: None, last_change: None,
                endpoint_id: endpoint.id.clone(), api_key_id: None, credential_binding: None,
                catalog_service_id: Some(catalog.id.clone()), icon_url: None, node_id: None,
                node_priority: 0, service_type: "http".into(), ssh_auth_mode: SshAuthMode::ProxyOnly,
                admin_only: false, ssh_node_keys_stale: false, auth_method: "none".into(),
                auth_key_name: String::new(), identity_propagation_mode: "none".into(),
                identity_include_user_id: false, identity_include_email: false, identity_include_name: false,
                identity_jwt_audience: None, forward_access_token: true, inject_delegation_token: false,
                delegation_token_scope: "llm:proxy".into(), custom_user_agent: None,
                default_request_headers: None, ws_frame_injections: vec![], is_active: true,
                source: Some(BOUND_SOURCE.into()), source_id: Some(agent.id.clone()), source_app_id: None,
                created_at: now, updated_at: now, state_version: 1, rotation_predecessor_id: None,
            };
            service_history::collection::<UserEndpoint>(&db, crate::models::user_endpoint::COLLECTION_NAME)
                .insert_one(endpoint).session(&mut *tx).await?;
            service_history::collection::<UserService>(&db, crate::models::user_service::COLLECTION_NAME)
                .insert_one(&service).session(&mut *tx).await?;
            Ok(service)
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
}
