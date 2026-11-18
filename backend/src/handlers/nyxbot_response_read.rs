//! Response reads observe settlement without acquiring delivery ownership.
use super::*;
use crate::services::nyxbot_response_read as read;

pub(super) async fn load(
    state: &AppState,
    row: &NyxbotChannel,
    id: &str,
) -> AppResult<Option<read::View>> {
    let Some(mut view) = read::load(&state.db, row, id).await? else {
        return Ok(None);
    };
    if !view.admitted {
        return Ok(None);
    }
    if let Some(ciphertext) = view.reply_ciphertext.take() {
        let payload = crate::services::nyxbot_response_recovery::open_payload(
            &state.encryption_keys,
            &ciphertext,
        )
        .await?;
        if payload.snapshot["id"] != view.payload.snapshot["id"]
            || payload.snapshot["conversation"] != view.payload.snapshot["conversation"]
            || payload.snapshot["metadata"] != view.payload.snapshot["metadata"]
            || payload.snapshot["status"] != "completed"
            || payload
                .frames
                .first()
                .map(|f| &f["response"]["thread_reply"])
                != view
                    .payload
                    .frames
                    .first()
                    .map(|f| &f["response"]["thread_reply"])
        {
            return Err(AppError::Internal("Response reply identity changed".into()));
        }
        view.payload = payload;
    }
    Ok(Some(view))
}

pub(super) fn stream(
    state: AppState,
    row: NyxbotChannel,
    headers: HeaderMap,
    id: String,
    after: Option<u64>,
) -> Response {
    let response_id = id.clone();
    let frames = async_stream::stream! {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(570);
        let mut next = after.map_or(0, |n| n + 1);
        loop {
            let live = provider_channel(&state, &headers).await;
            let view = match live {
                Ok((live, _)) if live.id == row.id && live.binding_id == row.binding_id =>
                    load(&state, &live, &id).await,
                _ => Err(AppError::OAuthAuthorityUnavailable),
            };
            match view {
                Ok(Some(view)) => {
                    for frame in view.payload.frames.into_iter().skip(next as usize) {
                        next = frame["sequence_number"].as_u64().expect("lifecycle sequence") + 1;
                        yield frame;
                    }
                    if view.terminal { break; }
                }
                _ => { yield json!({"type":"error","code":"provider_unavailable"}); break; }
            }
            if tokio::time::Instant::now() >= deadline { break; }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };
    let mut response = response_sse(frames, &response_id);
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().expect("static header"));
    response
}

/// A no-turn reply has one owner: the gateway parent, just like a stored
/// no-turn response. Body polling never consumes its encrypted recovery copy.
#[allow(clippy::too_many_arguments)]
pub(super) async fn reply(
    state: AppState,
    row: &NyxbotChannel,
    event: String,
    id: String,
    created: Value,
    text: String,
    thread_reply: bool,
) -> AppResult<Response> {
    use crate::models::nyxbot_response_read::ReplyPayload;
    use crate::services::nyxbot_response_recovery::Payload;
    let mut snapshot = created["response"].clone();
    snapshot
        .as_object_mut()
        .expect("created response")
        .remove("thread_reply");
    let mut payload = Payload {
        snapshot,
        frames: vec![created],
    };
    payload.finish_text(Some(&text), false);
    let reply = ReplyPayload {
        version: 1,
        response_id: id.clone(),
        ciphertext: payload.seal(&state.encryption_keys).await?,
    };
    let reply = bson::to_bson(&reply)
        .map_err(|_| AppError::Internal("Response reply unavailable".into()))?;
    let changed = state.db.collection::<bson::Document>(EVENTS).update_one(doc! {
        "_id":&event,"channel_id":&row.id,"response_read.version":1,
        "response_read.expires_at":{"$gt":bson::DateTime::now()},
        "turn_id":bson::Bson::Null,"response_reply":bson::Bson::Null,
        "status":{"$in":["running","resuming"]},
    }, doc! {"$set":{
        "status":"refused", "response_read.thread_reply":thread_reply, "response_reply":reply,
    },"$unset":{"event_context_ciphertext":""}}).await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict(
            "Response reply admission changed".into(),
        ));
    }
    Ok(response_sse(futures::stream::iter(payload.frames), &id))
}
