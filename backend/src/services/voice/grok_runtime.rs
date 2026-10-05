//! Grok starts on the authenticated relay replica; no provider audio in POST.
use super::{credentials, grok, runtime, session, transport::Transport};
use crate::{
    AppState,
    errors::{AppError, AppResult, voice_start::Stage},
    models::{
        assistant_voice::VoicePreferences,
        assistant_voice_session::{COLLECTION_NAME, VoiceSession},
        downstream_service::VoiceProtocol,
    },
};
use mongodb::bson::{self, doc};
use tokio::sync::mpsc;

pub async fn prepare(
    state: &AppState,
    user: &str,
    conversation: &str,
    request: &str,
    preferences: VoicePreferences,
    identity: String,
) -> AppResult<runtime::Started> {
    let row = session::admit(
        &state.db,
        user,
        conversation,
        request,
        preferences,
        identity,
        &state.replica_identity.generation_id,
    )
    .await?;
    let row = session::write(&state.db, &row, doc! {"protocol":"xai_realtime"}).await?;
    Ok(runtime::Started {
        session: row,
        sdp: String::new(),
    })
}

pub async fn serve(
    state: AppState,
    mut call: VoiceSession,
    socket_id: String,
    input: mpsc::Receiver<grok::ClientInput>,
    output: mpsc::Sender<grok::Output>,
) {
    let errors = output.clone();
    let setup = async {
        // A control stream can land on another replica. Its single-stream claim
        // transfers only an unstarted relay, never an established provider call.
        call = state.db.collection::<VoiceSession>(COLLECTION_NAME).find_one_and_update(
            doc! {"_id":&call.id,"generation":call.generation,"state":"starting","end_requested":false,
                "control_socket_id":&socket_id,"lease_until":{"$gt":bson::DateTime::now()}},
            doc! {"$set":{"lease_owner":&state.replica_identity.generation_id}})
            .return_document(mongodb::options::ReturnDocument::After).await?
            .ok_or_else(||AppError::Conflict("Voice relay already started".into()))?;
        let resolved = Box::pin(credentials::resolve(
            &state,
            &call.user_id,
            &call.conversation_id,
            &call.preferences,
        ))
        .await
        .map_err(|e| Stage::Credential.error(e))?;
        if resolved.protocol != VoiceProtocol::XaiRealtime
            || resolved.identity != call.credential_identity
        {
            return Err(Stage::Credential.error(AppError::VoiceProviderUnavailable));
        }
        call = session::refresh(&state.db, &call).await?;
        let meter = super::super::billing::voice::reserve(
            &state.db,
            &state.billing,
            resolved.billing.clone(),
            &call.id,
            0,
        )
        .await
        .map_err(|e| Stage::BillingReservation.error(e))?;
        call = session::write(&state.db, &call, doc! {"reserved_until":30})
            .await
            .map_err(|e| Stage::BillingReservation.error(e))?;
        let instructions = runtime::initial_context(&state.db, &call)
            .await
            .map_err(|e| Stage::Thread.error(e))?;
        state
            .billing
            .mark_forwarded(&meter)
            .await
            .map_err(|e| Stage::BillingReservation.error(e))?;
        let (socket, started) = grok::connect(
            &resolved.key,
            &call.preferences.model,
            &resolved.voice,
            &instructions,
        )
        .await?;
        let transport = Transport::Grok(Box::new(grok::Handle::spawn(
            grok::Grok::new(
                socket,
                input,
                output,
                state.clone(),
                call.clone(),
                resolved.token_billing,
            )
            .started_at(started),
        )));
        call = session::write(&state.db, &call, doc! {"state":"active"}).await?;
        Ok::<_, AppError>((transport, resolved.billing))
    };
    match tokio::time::timeout(std::time::Duration::from_secs(12), Box::pin(setup)).await {
        Ok(Ok((transport, billing))) => runtime::run(state, call, None, transport, billing).await,
        failed => {
            let error = match failed {
                Ok(Err(e)) => e,
                _ => AppError::VoiceProviderUnavailable,
            };
            let error = super::diagnostics::finish::<()>(
                &state.db,
                &call.user_id,
                Err(error),
                Stage::Transport,
            )
            .await
            .unwrap_err();
            let _ = errors.try_send(grok::Output::StartFailed(error.response_body()));
            // Failed setup drops the sole provider socket; no invented duration.
            if runtime::settle_windows(&state, &call, false).await.is_ok()
                && let Ok(updated) =
                    session::write(&state.db, &call, doc! {"billing_finalized":true}).await
            {
                call = updated;
            }
            let _ = session::close(&state.db, &call, "start_failed", true).await;
        }
    }
}
