//! A server-owned Voice connection selects the Bot for the service ingress.
use crate::{
    errors::{AppError, AppResult},
    models::user_service::UserService,
    mw::auth::AuthUser,
};
use mongodb::{Database, bson::doc};

pub fn service_url(base_url: &str, service_id: &str) -> AppResult<String> {
    let base = url::Url::parse(base_url)
        .map_err(|_| AppError::ValidationError("Invalid NyxID base URL".into()))?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(AppError::ValidationError("Invalid NyxID base URL".into()));
    }
    let url = format!(
        "{}/api/v1/proxy/nyxbot-voice/{service_id}",
        base.as_str().trim_end_matches('/')
    );
    crate::services::user_endpoint_service::validate_endpoint_url(&url)?;
    Ok(url)
}

pub struct Identity {
    pub service: UserService,
    pub agent: crate::models::assistant_agent::AssistantAgent,
}

pub async fn resolve(db: &Database, auth: &AuthUser, service: &str) -> AppResult<Identity> {
    let caller = super::client_key::caller(db, auth).await?;
    resolve_for_caller(db, &caller.auth, service).await
}

/// The caller has already been freshly authenticated for this request.
pub(crate) async fn resolve_for_caller(
    db: &Database,
    auth: &AuthUser,
    service: &str,
) -> AppResult<Identity> {
    let owner = auth.user_id.to_string();
    let row = super::access::connection(db, auth, service).await?;
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

pub async fn call(
    db: &Database,
    identity: &Identity,
    sid: &str,
) -> AppResult<crate::models::assistant_voice_session::VoiceSession> {
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
    Ok(call)
}
