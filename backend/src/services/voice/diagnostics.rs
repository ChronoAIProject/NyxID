//! One metadata-only diagnostic at each failed startup boundary.
use crate::{
    errors::{AppError, AppResult, voice_start::Stage},
    services::audit_service,
};

pub async fn finish<T>(
    db: &mongodb::Database,
    user: &str,
    result: AppResult<T>,
    fallback: Stage,
) -> AppResult<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let error = fallback.error(error);
            let AppError::VoiceStartFailed(failure) = &error else {
                unreachable!()
            };
            let details = failure.details();
            let provider = failure.provider.as_ref();
            tracing::warn!(
                event = "assistant_voice_start_failed", stage = failure.stage.as_str(),
                error_code = error.error_code(),
                user_id = user,
                provider = ?provider.map(|p|p.provider),
                provider_status = provider.and_then(|p|p.provider_status),
                provider_type = provider.and_then(|p|p.provider_type.as_deref()),
                provider_code = provider.and_then(|p|p.provider_code.as_deref()),
                provider_param = provider.and_then(|p|p.provider_param.as_deref()),
                "Voice start failed"
            );
            let mut metadata = details;
            metadata["error_code"] = serde_json::json!(error.error_code());
            let _ = audit_service::log_actor_event(
                db.clone(),
                &audit_service::AuditActor {
                    user_id: user.into(),
                    ip_address: None,
                    user_agent: None,
                    api_key_id: None,
                    api_key_name: None,
                },
                "assistant_voice_start_failed",
                Some(metadata),
            )
            .await;
            Err(error)
        }
    }
}

/// Handshake errors may include an upstream response; retain only bounded error identifiers.
pub fn socket_error(
    error: tokio_tungstenite::tungstenite::Error,
    provider: crate::errors::voice_start::Provider,
    stage: Stage,
) -> AppError {
    use crate::errors::voice_start::ProviderFailure;
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => ProviderFailure::new(
            provider,
            Some(response.status().as_u16()),
            response.body().as_deref().unwrap_or_default(),
        )
        .error(stage),
        _ => Stage::Transport.error(AppError::VoiceProviderUnavailable),
    }
}

/// No provider IDs, bodies or credential identity in the close audit.
pub async fn close_unconfirmed(
    db: &mongodb::Database,
    call: &crate::models::assistant_voice_session::VoiceSession,
) {
    let _ = audit_service::log_actor_event(
        db.clone(),
        &audit_service::AuditActor {
            user_id: call.user_id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "assistant_voice_close_unconfirmed",
        Some(serde_json::json!({
            "reason":"close_unconfirmed", "session_id":call.id, "generation":call.generation,
        })),
    )
    .await;
}
