//! A server-owned Voice connection selects the Bot for every ingress.
//! The URL is a non-routable capability marker; execution stays in this instance.
use crate::{
    errors::{AppError, AppResult},
    models::user_service::UserService,
    mw::auth::{AuthMethod, AuthUser},
};
use mongodb::{Database, bson::doc};
pub const INTERNAL_BASE_URL: &str = "https://nyxid.invalid/internal/nyxbot-voice";

pub struct Identity {
    pub service: UserService,
    pub agent: crate::models::assistant_agent::AssistantAgent,
}

pub async fn resolve(db: &Database, auth: &AuthUser, service: &str) -> AppResult<Identity> {
    let key = super::client_key::authenticate(db, auth).await?;
    let fresh;
    let auth = if let Some(key) = key {
        fresh = crate::mw::auth::api_key_auth_user(
            db,
            &key,
            auth.api_key_credential_id.clone(),
            None,
            None,
        )
        .await?;
        &fresh
    } else {
        auth
    };
    let owner = auth.user_id.to_string();
    let row = if auth.auth_method == AuthMethod::ApiKey {
        super::access::services(db, auth)
            .await?
            .into_iter()
            .find(|row| row.id == service)
    } else {
        db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
            .find_one(doc! {"_id":service,"user_id":&owner,"is_active":true,
            "deleted_at":mongodb::bson::Bson::Null,"source":super::access::BOUND_SOURCE})
            .await?
    }
    .ok_or_else(|| {
        AppError::ApiKeyScopeForbidden("This Key has no access to this Voice connection".into())
    })?;
    let agent = crate::services::assistant_team_service::agent(
        db,
        &owner,
        row.source_id.as_deref().unwrap_or_default(),
    )
    .await?;
    if agent.user_id != owner || agent.destroyed_at.is_some() {
        return Err(AppError::ApiKeyScopeForbidden(
            "Voice Bot is unavailable".into(),
        ));
    }
    Ok(Identity {
        service: row,
        agent,
    })
}

pub async fn conversation(db: &Database, identity: &Identity, conversation: &str) -> AppResult<()> {
    let thread =
        crate::services::assistant_voice::thread(db, &identity.service.user_id, conversation)
            .await?;
    let agent =
        crate::services::assistant_team_service::agent_for_conversation(db, &thread).await?;
    if agent.id != identity.agent.id || thread.shared_agent_thread {
        return Err(AppError::ApiKeyScopeForbidden(
            "Conversation belongs to another Voice connection".into(),
        ));
    }
    Ok(())
}

pub async fn call(db: &Database, identity: &Identity, sid: &str) -> AppResult<()> {
    let call = db
        .collection::<crate::models::assistant_voice_session::VoiceSession>(
            crate::models::assistant_voice_session::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":sid,"user_id":&identity.service.user_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Voice session not found".into()))?;
    conversation(db, identity, &call.conversation_id).await?;
    if call.client_key.as_ref().is_some_and(|key| {
        key.service_id != identity.service.id || key.agent_id != identity.agent.id
    }) {
        return Err(AppError::ApiKeyScopeForbidden(
            "Call belongs to another Voice connection".into(),
        ));
    }
    Ok(())
}
