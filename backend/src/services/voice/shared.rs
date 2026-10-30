//! Exact private connection consent for one shared voice session.
use crate::services::{
    shared_agent_consent as consent, shared_agent_service as sharing,
    shared_agent_transport as transport,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_sharing::SharedExecution, assistant_voice::VoicePreferences,
        assistant_voice_session::VoiceSession,
    },
};
use mongodb::Database;

pub(crate) fn connection(
    access: &sharing::RequestAccess,
    catalog: &str,
    selected: Option<&str>,
) -> AppResult<crate::models::user_service::UserService> {
    let connections = access.catalog_connections(catalog, true);
    if let Some(id) = selected {
        return connections
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(sharing::missing);
    }
    let mut platforms = connections.into_iter().filter(|s| {
        s.user_id == access.actor()
            && (s.credential_binding.as_deref() == Some("platform") || s.api_key_id.is_none())
    });
    let service = platforms.next().ok_or_else(sharing::missing)?;
    if platforms.next().is_some() {
        return Err(AppError::Conflict(
            "Select an exact voice connection".into(),
        ));
    }
    Ok(service)
}

pub async fn admit(
    db: &Database,
    user: &str,
    thread: &crate::models::assistant_conversation::AssistantConversation,
    request: &str,
    preferences: &VoicePreferences,
) -> AppResult<Option<SharedExecution>> {
    if !thread.shared_agent_thread {
        return Ok(None);
    }
    transport::require_ready(db).await?;
    let auth = crate::mw::auth::api_key_auth_user_for_voice(db, thread).await?;
    let access = auth
        .org_agent_access
        .as_ref()
        .and_then(|a| a.shared())
        .ok_or_else(sharing::missing)?;
    let service = connection(
        access,
        &preferences.service_id,
        (preferences.key_source == crate::models::assistant_voice::VoiceKeySource::Own)
            .then_some(preferences.connection_id.as_deref())
            .flatten(),
    )?;
    if service.user_id != user
        && access.level(&service.id) != Some(crate::models::assistant_agent::GuestAccess::All)
    {
        return Err(AppError::Forbidden(
            "Shared voice requires explicit full service access".into(),
        ));
    }
    let binding = crate::services::assistant_service_execution::connection_binding(
        db,
        &service.id,
        &thread.credential_api_key_id,
    )
    .await?;
    let mut chat = auth
        .assistant_chat
        .as_deref()
        .cloned()
        .ok_or_else(sharing::missing)?;
    // A session identity scopes the private consent; this does not make a turn live.
    chat.turn_id = Some(request.into());
    let operation = serde_json::json!({"kind":"voice_session", "preferences":preferences});
    let permit = if service.user_id == user {
        Some(consent::authorize_voice(db, &chat, access, &service, &binding, &operation).await?)
    } else {
        None
    };
    let (consent_id, consent_claim) = permit
        .as_ref()
        .map(|p| {
            let (id, claim) = p.durable();
            (Some(id), claim)
        })
        .unwrap_or_default();
    let row = SharedExecution {
        voice: true,
        model: true,
        actor: user.into(),
        owner: access.owner().into(),
        agent: chat.agent_id,
        conversation: thread.id.clone(),
        key: thread.credential_api_key_id.clone(),
        turn: request.into(),
        connection: service.id,
        revision: access.revision(),
        behavior: consent::behavior(access),
        binding,
        consent_id,
        consent_claim,
    };
    transport::revalidate(db, &row).await?;
    Ok(Some(row))
}

pub async fn revalidate(db: &Database, call: &VoiceSession) -> AppResult<()> {
    if let Some(binding) = &call.shared_execution {
        if binding.actor != call.user_id
            || binding.conversation != call.conversation_id
            || binding.turn != call.client_request_id
        {
            return Err(sharing::missing());
        }
        transport::revalidate(db, binding).await?;
    }
    Ok(())
}

pub async fn before_start(state: &crate::AppState, call: &VoiceSession) -> AppResult<()> {
    if call.shared_execution.is_some() {
        revalidate(&state.db, call).await?;
        let current = Box::pin(super::credentials::resolve(
            state,
            &call.user_id,
            &call.conversation_id,
            &call.preferences,
        ))
        .await?;
        if current.identity != call.credential_identity {
            return Err(sharing::missing());
        }
        revalidate(&state.db, call).await?;
    }
    Ok(())
}
