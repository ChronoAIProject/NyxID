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
        crate::mw::auth::require_first_party_human(auth)?;
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
pub async fn resolve(
    db: &Database,
    auth: &AuthUser,
    conversation: &str,
) -> AppResult<Option<ClientKey>> {
    Ok(resolve_authority(db, auth, conversation)
        .await?
        .map(|(binding, _)| binding))
}

async fn resolve_authority(
    db: &Database,
    auth: &AuthUser,
    conversation: &str,
) -> AppResult<Option<(ClientKey, AuthUser)>> {
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
    Ok(Some((
        ClientKey {
            key_id: key.id,
            credential_id: auth.api_key_credential_id.clone(),
            key_version: key.state_version,
            agent_id: agent.id,
            service_id: service.id,
            service_version: service.state_version,
        },
        fresh,
    )))
}
/// Start admission composes the exact Key grant with the Bot Voice path.
pub async fn authorize(
    db: &Database,
    auth: &AuthUser,
    conversation: &str,
) -> AppResult<Option<ClientKey>> {
    let Some((binding, fresh)) = resolve_authority(db, auth, conversation).await? else {
        return Ok(None);
    };
    let service = bound_service(db, &binding.service_id).await?;
    super::access::policy(db, &fresh, &service, "POST", "/voice/sessions", true).await?;
    Ok(Some(binding))
}

async fn bound_service(
    db: &Database,
    id: &str,
) -> AppResult<crate::models::user_service::UserService> {
    db.collection(crate::models::user_service::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(refused)
}

/// Authorize the current Voice operation without creating service approvals.
pub async fn operation(
    db: &Database,
    auth: &AuthUser,
    conversation: &str,
    method: &str,
    path: &str,
    websocket: bool,
) -> AppResult<()> {
    let Some((binding, fresh)) = resolve_authority(db, auth, conversation).await? else {
        return Ok(());
    };
    let service = bound_service(db, &binding.service_id).await?;
    super::access::policy(db, &fresh, &service, method, path, websocket).await
}

pub async fn call_operation(
    db: &Database,
    call: &VoiceSession,
    method: &str,
    path: &str,
    websocket: bool,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let Some(binding) = &call.client_key else {
        return Ok(());
    };
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":&binding.key_id,"user_id":&call.user_id})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    ordinary(&key)?;
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        db,
        &key,
        binding.credential_id.clone(),
        None,
        None,
    ))
    .await?;
    // fence_call has locked the authoritative identity and catalog rows in
    // this transaction. Reuse its snapshot for the exact operation source.
    let service = db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .find_one(doc! {"_id":&binding.service_id,"user_id":&call.user_id})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    super::access::policy(db, &auth, &service, method, path, websocket).await
}

pub async fn revalidate(db: &Database, call: &VoiceSession) -> AppResult<()> {
    revalidate_binding(db, call, false).await.map(|_| ())
}

/// Capture the currently authorized versions for a transactional fence. Key
/// state_version also changes on last_used_at; it is not a durable call identity.
async fn revalidate_binding(
    db: &Database,
    call: &VoiceSession,
    in_transaction: bool,
) -> AppResult<Option<ClientKey>> {
    let Some(binding) = &call.client_key else {
        return Ok(None);
    };
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":&binding.key_id,"user_id":&call.user_id})
        .await?
        .ok_or_else(refused)?;
    ordinary(&key)?;
    if !in_transaction
        && !crate::services::feature_flag_service::personal_flag_enabled(
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
    if service.id != binding.service_id {
        return Err(refused());
    }
    if !in_transaction {
        super::access::policy(db, &fresh, &service, "POST", "/voice/sessions", true).await?;
    }
    let mut current = binding.clone();
    current.key_version = key.state_version;
    Ok(Some(current))
}
pub async fn own_call(db: &Database, auth: &AuthUser, call: &VoiceSession) -> AppResult<()> {
    if let Some(proof) = resolve(db, auth, &call.conversation_id).await? {
        if !call.client_key.as_ref().is_some_and(|bound| {
            bound.key_id == proof.key_id
                && bound.credential_id == proof.credential_id
                && bound.agent_id == proof.agent_id
                && bound.service_id == proof.service_id
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
    if request.state == crate::models::assistant_voice::RequestState::Cancelled {
        return Err(refused());
    }
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
    let row = db
        .collection::<crate::models::assistant_voice::VoiceRequest>(
            crate::models::assistant_voice::REQUESTS,
        )
        .find_one_and_update(
            doc! {"_id":request_id,"user_id":user},
            doc! {"$inc":{"source_authority_fence":1_i64}},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    if row.state == crate::models::assistant_voice::RequestState::Cancelled {
        return Err(refused());
    }
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
    let Some(binding) = &call.client_key else {
        return Ok(());
    };
    let expiry =
        doc! {"$or":[{"expires_at":bson::Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]};
    let key_filter = doc! {"_id":&binding.key_id,"user_id":user,"is_active":true,
    "$and":[expiry.clone()]};
    // Write the identity rows from this transaction's snapshot *before* live
    // resolution. A newer authentication last_used_at write is a Mongo retry,
    // never a failed comparison against a version read outside the snapshot.
    let service = db
        .collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&binding.service_id,"user_id":user})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    let mut fences = vec![
        (
            crate::models::assistant_conversation::COLLECTION_NAME,
            doc! {"_id":&call.conversation_id,"user_id":user,"shared_agent_thread":{"$ne":true}},
        ),
        (KEYS, key_filter),
        (
            crate::models::user::COLLECTION_NAME,
            doc! {"_id":user,"is_active":true,"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}]},
        ),
        (
            crate::models::downstream_service::COLLECTION_NAME,
            doc! {"_id":service.catalog_service_id.as_deref().unwrap_or_default(),"is_active":true},
        ),
        (
            crate::models::assistant_agent::COLLECTION_NAME,
            doc! {"_id":&binding.agent_id,"user_id":user,"destroyed_at":bson::Bson::Null},
        ),
        (
            crate::models::user_service::COLLECTION_NAME,
            doc! {"_id":&binding.service_id,"user_id":user,"is_active":true,
            "source":super::access::BOUND_SOURCE,"source_id":&binding.agent_id,
            "deleted_at":bson::Bson::Null},
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
    // These reads now observe the same committed authority as the locked
    // snapshot rows. Revocation either preceded this attempt or conflicts with
    // it; metadata-only Key updates cannot become stable call ownership.
    Box::pin(revalidate_binding(db, call, true)).await?;
    for flag in [
        crate::services::feature_flag_service::VOICE_AGENT_KEY_FLAG_KEY,
        crate::services::feature_flag_service::ASSISTANT_VOICE_FLAG_KEY,
    ] {
        if !crate::services::feature_flag_service::personal_flag_enabled_in_session(
            db, user, flag, session,
        )
        .await?
        {
            return Err(refused());
        }
    }
    call_operation(db, call, "POST", "/voice/sessions", true, session).await?;
    Ok(())
}
pub async fn end(db: &Database, call: &VoiceSession) -> AppResult<()> {
    db.collection::<bson::Document>(CALLS).update_one(
        doc! {"_id":&call.id,"user_id":&call.user_id,"live_slot":true},
        doc! {"$set":{"end_requested":true,"state":"closing","end_reason":"client_key_invalid"},"$inc":{"control_revision":1}},
    ).await?;
    db.collection::<bson::Document>(super::announcements::COLLECTION)
        .update_many(
            doc! {"session_id":&call.id,"user_id":&call.user_id,"state":"queued"},
            doc! {"$set":{"state":"cancelled"}},
        )
        .await?;
    use futures::TryStreamExt;
    let pending: Vec<crate::models::assistant_voice::VoiceRequest> = db
        .collection(crate::models::assistant_voice::REQUESTS)
        .find(doc! {"session_id":&call.id,"user_id":&call.user_id})
        .await?
        .try_collect()
        .await?;
    for row in pending {
        assistant_voice::cancel(db, &row.user_id, &row.conversation_id, &row.id).await?;
    }
    Ok(())
}

/// Only confirmed loss of source authority is destructive. Storage, readiness,
/// provider and concurrency failures preserve durable work for bounded retry.
pub fn permanent_authority_failure(error: &AppError) -> bool {
    error.is_forbidden()
        || matches!(
            error,
            AppError::Unauthorized(_)
                | AppError::ApiKeyScopeForbidden(_)
                | AppError::NotFound(_)
                | AppError::TokenExpired
        )
}

/// A descendant retains its causing source while executing with its own Bot key.
pub fn turn_source(turn: &crate::models::assistant_conversation::ActiveTurn) -> Option<&str> {
    turn.voice_request_id
        .as_deref()
        .or(turn.voice_source_request_id.as_deref())
}

pub async fn source_for_chat(
    db: &Database,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
) -> AppResult<Option<String>> {
    let row = db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,
        "credential_api_key_id":&chat.api_key_id})
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    // Ordinary tool dispatch keeps its existing live-turn rules. This helper
    // adds a fence only when the current turn actually carries Voice authority.
    let source = row
        .active_turn
        .as_ref()
        .and_then(turn_source)
        .map(str::to_owned);
    if let Some(source) = &source {
        if row.active_turn.as_ref().is_none_or(|turn| {
            Some(turn.turn_id.as_str()) != chat.turn_id.as_deref() || turn.stop_requested
        }) {
            return Err(AppError::AssistantTurnRequired);
        }
        Box::pin(request(db, &chat.user_id, source)).await?;
    }
    Ok(source)
}

/// Delayed reports retain their causing authority. Remove only confirmed lost
/// sources; uncertainty aborts admission without consuming any queued event.
pub async fn filter_events_in_session(
    db: &Database,
    user: &str,
    events: Vec<crate::models::assistant_conversation::AgentEvent>,
    session: &mut mongodb::ClientSession,
) -> AppResult<Vec<crate::models::assistant_conversation::AgentEvent>> {
    let mut eligible = Vec::new();
    for event in events {
        if let Some(source) = &event.voice_source_request_id {
            match Box::pin(fence_request(db, user, source, session)).await {
                Ok(()) => {}
                Err(error) if permanent_authority_failure(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        eligible.push(event);
    }
    Ok(eligible)
}
