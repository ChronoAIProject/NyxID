//! Human-only voice control and durable request dispatch; no media adapter yet.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_voice::{VoiceInputMode, VoiceKeySource, VoicePreferences},
    mw::auth::AuthUser,
    services::{assistant_nyxagent as engine, assistant_voice as voice},
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub service_id: String,
    pub connection_id: Option<String>,
    pub key_source: VoiceKeySource,
    pub model: String,
    pub voice: Option<String>,
    #[serde(default)]
    pub input_mode: VoiceInputMode,
    pub language: Option<String>,
    #[serde(default)]
    pub notify_on_completion: bool,
}
impl From<VoicePreferences> for Preferences {
    fn from(v: VoicePreferences) -> Self {
        Self {
            service_id: v.service_id,
            connection_id: v.connection_id,
            key_source: v.key_source,
            model: v.model,
            voice: v.voice,
            input_mode: v.input_mode,
            language: v.language,
            notify_on_completion: v.notify_on_completion,
        }
    }
}
impl From<Preferences> for VoicePreferences {
    fn from(v: Preferences) -> Self {
        Self {
            service_id: v.service_id,
            connection_id: v.connection_id,
            key_source: v.key_source,
            model: v.model,
            voice: v.voice,
            input_mode: v.input_mode,
            language: v.language,
            notify_on_completion: v.notify_on_completion,
        }
    }
}
pub fn preference_patch<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Preferences>>, D::Error> {
    Option::<Preferences>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionsQuery {
    conversation_id: String,
}

pub async fn options(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<OptionsQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let user = auth.user_id.to_string();
    voice::require_enabled(&state.db, &user).await?;
    let thread = voice::thread(&state.db, &user, &q.conversation_id).await?;
    let options = voice::options(&state.db, &state.encryption_keys, &thread).await?;
    Ok(Json(serde_json::json!({"options":options,"limits": {
        "call_seconds":crate::models::assistant_voice::MAX_CALL_SECONDS,"idle_seconds":crate::models::assistant_voice::IDLE_SECONDS,"idle_warning_seconds":crate::models::assistant_voice::IDLE_WARNING_SECONDS,
        "transmitting_calls":1,"queued_requests":crate::models::assistant_voice::MAX_QUEUED},"recording":false})))
}

pub async fn start(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<()> {
    super::login_client_context::require_first_party_human(&auth)?;
    let user = auth.user_id.to_string();
    voice::require_enabled(&state.db, &user).await?;
    voice::thread(&state.db, &user, &id).await?;
    // Flag enable alone cannot accidentally start unmetered provider sessions.
    Err(AppError::VoiceProviderUnavailable)
}

#[derive(Serialize)]
pub struct RequestResponse {
    pending_acknowledgement_ids: Vec<String>,
    id: String,
    state: crate::models::assistant_voice::RequestState,
    turn_id: String,
}
pub async fn request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, rid)): Path<(String, String)>,
) -> AppResult<Json<RequestResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let user = auth.user_id.to_string();
    let row = voice::get(&state.db, &user, &id, &rid).await?;
    Ok(Json(RequestResponse {
        pending_acknowledgement_ids: row.pending_acknowledgement_ids,
        id: row.id,
        state: row.state,
        turn_id: row.turn_id,
    }))
}
pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, rid)): Path<(String, String)>,
) -> AppResult<()> {
    super::login_client_context::require_first_party_human(&auth)?;
    voice::cancel(&state.db, &auth.user_id.to_string(), &id, &rid).await
}

/// The normal owner limiter, turn fence and egress policy apply to queued speech.
/// Sweeps retry admission only; a claimed turn is never replayed after a crash.
pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    use crate::models::{
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        assistant_voice::REQUESTS,
    };
    voice::recover(&state.db).await?;
    let rows = voice::queued(&state.db).await?;
    for request in rows {
        let result: AppResult<()> = Box::pin(async {
            if request.expires_at <= chrono::Utc::now() {
                return Err(AppError::Conflict("Voice request expired".into()));
            }
            voice::thread(&state.db, &request.user_id, &request.conversation_id).await?;
            let message = state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
                "_id":&request.message_id,"user_id":&request.user_id,"conversation_id":&request.conversation_id,
                "turn_id":&request.turn_id,"execution_pending":true,
            }).await?.ok_or_else(|| AppError::NotFound("Voice input unavailable".into()))?;
            let start = engine::TurnStart::from(&engine::TurnRequest {
                conversation_id:Some(request.conversation_id.clone()),text:message.text,
                model:None,agent_id:None,attachment_ids:Vec::new(),access_mode:None,
            });
            let permit = state.direct_chat_limiter.try_acquire(&request.user_id).await?;
            Box::pin(super::assistant_nyxagent::start_turn_with_voice(state,
                super::assistant_team::owner_auth(&request.user_id)?, &start,
                Some(super::assistant_nyxagent::SERVER_TURN_POLICY),permit,Some(&request.id))).await?;
            Ok(())
        }).await;
        if matches!(
            result,
            Err(AppError::NotFound(_) | AppError::Forbidden(_) | AppError::Conflict(_))
        ) {
            state
                .db
                .collection::<crate::models::assistant_voice::VoiceRequest>(REQUESTS)
                .update_one(
                    doc! {"_id":&request.id,"state":"queued"},
                    doc! {"$set":{"state":"cancelled"}},
                )
                .await?;
        }
        // Pool/active-turn/temporary database errors leave the durable queue intact.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_preserve_omitted_and_reset_explicit_null() {
        #[derive(Deserialize)]
        struct Patch {
            #[serde(default, deserialize_with = "preference_patch")]
            voice: Option<Option<Preferences>>,
        }
        assert!(serde_json::from_str::<Patch>("{}").unwrap().voice.is_none());
        assert!(matches!(
            serde_json::from_str::<Patch>(r#"{"voice":null}"#)
                .unwrap()
                .voice,
            Some(None)
        ));
        let value = serde_json::json!({"voice":{
            "service_id":uuid::Uuid::new_v4().to_string(),"connection_id":null,
            "key_source":"platform","model":"gpt-live-1","voice":null,"language":null
        }});
        let patch: Patch = serde_json::from_value(value).unwrap();
        let mut preferences: VoicePreferences = patch.voice.unwrap().unwrap().into();
        voice::validate_preferences(&preferences).unwrap();
        preferences.key_source = VoiceKeySource::Own;
        assert!(voice::validate_preferences(&preferences).is_err());
        preferences.connection_id = Some(uuid::Uuid::new_v4().to_string());
        voice::validate_preferences(&preferences).unwrap();
        preferences.model = "gpt-live-1-mini".into();
        assert!(voice::validate_preferences(&preferences).is_err());
    }
}
