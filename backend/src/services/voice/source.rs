//! Voice adapter for the common execution-source contract.
use super::client_key::{fence_call, revalidate};
use crate::{
    errors::{AppError, AppResult},
    models::assistant_voice_session::{COLLECTION_NAME as CALLS, VoiceSession},
};
use mongodb::{Database, bson::doc};
fn refused() -> AppError {
    AppError::ApiKeyScopeForbidden("Execution source authority lost".into())
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
