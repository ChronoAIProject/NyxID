//! Ordinary Agent Key admission. Reuses existing key scopes and Bot grants;
//! never turns a service-scoped credential into an owner session.
pub use crate::models::assistant_voice_session::VoiceClientKey as ClientKey;
use crate::{
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS},
        api_key_credential::{ApiKeyCredential, COLLECTION_NAME as CREDENTIALS},
        assistant_agent::AssistantAgent,
        assistant_voice_session::{COLLECTION_NAME as CALLS, VoiceSession},
    },
    mw::auth::{AuthMethod, AuthUser},
    services::{assistant_team_service as team, assistant_voice, key_service},
};
use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
};
fn refused() -> AppError {
    AppError::ApiKeyScopeForbidden("This Key cannot access the selected Bot voice service".into())
}
fn ordinary(key: &ApiKey) -> AppResult<()> {
    if !key.is_active || key.expires_at.is_some_and(|at| at <= Utc::now()) {
        return Err(AppError::Unauthorized(
            "Voice Key is revoked or expired".into(),
        ));
    }
    if key.purpose != ApiKeyPurpose::General
        || key.assistant_agent_owner_id.is_some()
        || key.assistant_group_id.is_some()
        || crate::mw::auth::is_assistant_conversation_key_candidate(key)
        || !key
            .scopes
            .split_whitespace()
            .any(|s| matches!(s, "proxy" | "proxy:*"))
    {
        return Err(refused());
    }
    Ok(())
}

/// Discovery uses the same existing service grant and B1 operation checks.
pub async fn covers(db: &Database, key: &ApiKey, agent: &AssistantAgent) -> AppResult<()> {
    ordinary(key)?;
    if agent.user_id != key.user_id || agent.destroyed_at.is_some() {
        return Err(refused());
    }
    let auth = crate::mw::auth::api_key_auth_user(db, key, None, None, None).await?;
    super::access::authorize(db, &auth, agent).await?;
    Ok(())
}

pub async fn authenticate(db: &Database, auth: &AuthUser) -> AppResult<Option<ApiKey>> {
    if auth.auth_method != AuthMethod::ApiKey {
        crate::handlers::login_client_context::require_first_party_human(auth)?;
        return Ok(None);
    }
    if auth.assistant_chat.is_some() || auth.assistant_turn_fence.is_some() {
        return Err(refused());
    }
    let key = key_service::get_api_key(
        db,
        &auth.user_id.to_string(),
        auth.api_key_id.as_deref().ok_or_else(refused)?,
    )
    .await?;
    ordinary(&key)?;
    if !crate::services::feature_flag_service::personal_flag_enabled(
        db,
        &key.user_id,
        crate::services::feature_flag_service::VOICE_AGENT_KEY_FLAG_KEY,
    )
    .await?
    {
        return Err(AppError::Forbidden("Agent Key voice is not enabled".into()));
    }
    // Reuse authentication's live user/organization checks, not just a key row.
    let fresh = crate::mw::auth::api_key_auth_user(
        db,
        &key,
        auth.api_key_credential_id.clone(),
        None,
        None,
    )
    .await?;
    if fresh.org_agent_access.is_some() {
        return Err(refused());
    }
    child(db, &key, auth.api_key_credential_id.as_deref()).await?;
    Ok(Some(key))
}

/// Native and proxied entry points check the same published service operation.
pub async fn operation(
    db: &Database,
    auth: &AuthUser,
    conversation: &str,
    method: &str,
    path: &str,
    websocket: bool,
) -> AppResult<()> {
    let Some(key) = authenticate(db, auth).await? else {
        return Ok(());
    };
    let fresh = crate::mw::auth::api_key_auth_user(
        db,
        &key,
        auth.api_key_credential_id.clone(),
        None,
        None,
    )
    .await?;
    let thread = assistant_voice::thread(db, &key.user_id, conversation).await?;
    if thread.shared_agent_thread {
        return Err(refused());
    }
    let agent = team::agent_for_conversation(db, &thread).await?;
    let service = super::access::authorize(db, &fresh, &agent).await?;
    super::access::operation(db, &fresh, &service, method, path, websocket).await
}
async fn child(db: &Database, key: &ApiKey, id: Option<&str>) -> AppResult<()> {
    if let Some(id) = id {
        let row = db
            .collection::<ApiKeyCredential>(CREDENTIALS)
            .find_one(doc! {"_id":id,"api_key_id":&key.id,"user_id":&key.user_id,"is_active":true})
            .await?
            .ok_or_else(|| AppError::Unauthorized("Voice login credential revoked".into()))?;
        if row.expires_at.is_some_and(|at| at <= Utc::now()) {
            return Err(AppError::Unauthorized(
                "Voice login credential expired".into(),
            ));
        }
    }
    Ok(())
}
pub async fn authorize(
    db: &Database,
    auth: &AuthUser,
    conversation: &str,
) -> AppResult<Option<ClientKey>> {
    let Some(key) = authenticate(db, auth).await? else {
        return Ok(None);
    };
    let thread = assistant_voice::thread(db, &key.user_id, conversation).await?;
    if thread.shared_agent_thread
        || thread
            .agent_owner_id
            .as_deref()
            .is_some_and(|id| id != key.user_id)
    {
        return Err(refused());
    }
    let agent = team::agent_for_conversation(db, &thread).await?;
    let fresh = crate::mw::auth::api_key_auth_user(
        db,
        &key,
        auth.api_key_credential_id.clone(),
        None,
        None,
    )
    .await?;
    let service = super::access::authorize(db, &fresh, &agent).await?;
    Ok(Some(ClientKey {
        key_id: key.id,
        credential_id: auth.api_key_credential_id.clone(),
        key_version: key.state_version,
        agent_id: agent.id,
        service_id: service.id,
        service_version: service.state_version,
    }))
}
pub async fn revalidate(db: &Database, call: &VoiceSession) -> AppResult<()> {
    revalidate_binding(db, call).await.map(|_| ())
}

/// Capture the currently authorized versions for a transactional fence. Key
/// state_version also changes on last_used_at; it is not a durable call identity.
async fn revalidate_binding(db: &Database, call: &VoiceSession) -> AppResult<Option<ClientKey>> {
    let Some(binding) = &call.client_key else {
        return Ok(None);
    };
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":&binding.key_id,"user_id":&call.user_id})
        .await?
        .ok_or_else(refused)?;
    ordinary(&key)?;
    if !crate::services::feature_flag_service::personal_flag_enabled(
        db,
        &key.user_id,
        crate::services::feature_flag_service::VOICE_AGENT_KEY_FLAG_KEY,
    )
    .await?
    {
        return Err(refused());
    }
    child(db, &key, binding.credential_id.as_deref()).await?;
    let fresh = Box::pin(crate::mw::auth::api_key_auth_user(
        db,
        &key,
        binding.credential_id.clone(),
        None,
        None,
    ))
    .await?;
    if fresh.org_agent_access.is_some() {
        return Err(refused());
    }
    let thread = assistant_voice::thread(db, &call.user_id, &call.conversation_id).await?;
    if thread.shared_agent_thread {
        return Err(refused());
    }
    let agent = team::agent_for_conversation(db, &thread).await?;
    if agent.id != binding.agent_id {
        return Err(refused());
    }
    let service = super::access::authorize(db, &fresh, &agent).await?;
    if service.id != binding.service_id || service.state_version != binding.service_version {
        return Err(refused());
    }
    let mut current = binding.clone();
    current.key_version = key.state_version;
    Ok(Some(current))
}
pub async fn own_call(db: &Database, auth: &AuthUser, call: &VoiceSession) -> AppResult<()> {
    if let Some(proof) = authorize(db, auth, &call.conversation_id).await? {
        if !call.client_key.as_ref().is_some_and(|bound| {
            bound.key_id == proof.key_id
                && bound.credential_id == proof.credential_id
                && bound.agent_id == proof.agent_id
                && bound.service_id == proof.service_id
                && bound.service_version == proof.service_version
        }) {
            return Err(refused());
        }
        Box::pin(revalidate(db, call)).await?;
    }
    Ok(())
}
pub async fn request(db: &Database, user: &str, request_id: &str) -> AppResult<()> {
    let request = db
        .collection::<crate::models::assistant_voice::VoiceRequest>(
            crate::models::assistant_voice::REQUESTS,
        )
        .find_one(doc! {"_id":request_id,"user_id":user})
        .await?
        .ok_or_else(refused)?;
    if let Some(call) = db
        .collection::<VoiceSession>(CALLS)
        .find_one(doc! {"_id":&request.session_id,"user_id":user})
        .await?
    {
        if request.client_key != call.client_key {
            return Err(refused());
        }
        Box::pin(revalidate(db, &call)).await?;
    } else if request.client_key.is_some() {
        return Err(refused());
    }
    Ok(())
}

/// Join the action's existing transaction, so revoking a Key/child credential,
/// changing Bot grants, or disconnecting the service conflicts with admission.
pub async fn fence_request(
    db: &Database,
    user: &str,
    request_id: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    request(db, user, request_id).await?;
    let row = db
        .collection::<crate::models::assistant_voice::VoiceRequest>(
            crate::models::assistant_voice::REQUESTS,
        )
        .find_one(doc! {"_id":request_id,"user_id":user})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    let Some(original) = row.client_key else {
        return Ok(());
    };
    let call = db
        .collection::<VoiceSession>(CALLS)
        .find_one(doc! {"_id":&row.session_id,"user_id":user})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    if call.client_key.as_ref() != Some(&original) {
        return Err(refused());
    }
    fence_call(db, &call, session).await
}

/// Join any authenticated call mutation to the same Key/child/service fences.
pub async fn fence_call(
    db: &Database,
    call: &VoiceSession,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let user = call.user_id.as_str();
    let Some(binding) = Box::pin(revalidate_binding(db, call)).await? else {
        return Ok(());
    };
    let expiry =
        doc! {"$or":[{"expires_at":bson::Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]};
    let key_filter = doc! {"_id":&binding.key_id,"user_id":user,"is_active":true,
    "$and":[version_filter(binding.key_version), expiry.clone()]};
    let mut fences = vec![
        (KEYS, key_filter),
        (
            crate::models::assistant_agent::COLLECTION_NAME,
            doc! {"_id":&binding.agent_id,"user_id":user,"destroyed_at":bson::Bson::Null},
        ),
        (
            crate::models::user_service::COLLECTION_NAME,
            doc! {"_id":&binding.service_id,"user_id":user,"is_active":true,
            "source":super::access::BOUND_SOURCE,"source_id":&binding.agent_id,
            "$and":[version_filter(binding.service_version)]},
        ),
    ];
    if let Some(id) = &binding.credential_id {
        let mut child_filter =
            doc! {"_id":id,"api_key_id":&binding.key_id,"user_id":user,"is_active":true};
        child_filter.extend(expiry);
        fences.push((CREDENTIALS, child_filter));
    }
    for (collection, filter) in fences {
        if db
            .collection::<bson::Document>(collection)
            .update_one(filter, doc! {"$inc":{"voice_authority_fence":1_i64}})
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(refused());
        }
    }
    Ok(())
}
fn version_filter(version: i64) -> bson::Document {
    if version == 0 {
        doc! {"$or":[{"state_version":0_i64},{"state_version":{"$exists":false}}]}
    } else {
        doc! {"state_version":version}
    }
}
pub async fn end(db: &Database, call: &VoiceSession) -> AppResult<()> {
    db.collection::<bson::Document>(CALLS).update_one(
        doc! {"_id":&call.id,"user_id":&call.user_id,"live_slot":true},
        doc! {"$set":{"end_requested":true,"state":"closing","end_reason":"client_key_invalid"},"$inc":{"control_revision":1}},
    ).await?;
    use futures::TryStreamExt;
    let pending: Vec<crate::models::assistant_voice::VoiceRequest> = db.collection(crate::models::assistant_voice::REQUESTS)
        .find(doc! {"session_id":&call.id,"user_id":&call.user_id,"state":{"$in":["queued","claimed","awaiting_confirmation"]}})
        .limit(100).await?.try_collect().await?;
    for row in pending {
        assistant_voice::cancel(db, &row.user_id, &row.conversation_id, &row.id).await?;
    }
    Ok(())
}
