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
    if let Some((event_id, reply)) = view.reply.take() {
        let payload = crate::services::nyxbot_response_recovery::event_reply(
            &state.encryption_keys,
            &event_id,
            row,
            id,
            &reply,
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
        if let Some(digest) = &view.notice_digest {
            let text = payload.snapshot["output"][0]["content"][0]["text"]
                .as_str()
                .filter(|text| crate::crypto::token::hash_token(text) == *digest)
                .ok_or_else(|| AppError::Internal("Response notice changed".into()))?;
            // Replay the same exact text through the renderer. It verifies each
            // emitted frame as well as the snapshot, including the thread choice.
            let created = view.payload.frames[0].clone();
            let mut snapshot = created["response"].clone();
            snapshot
                .as_object_mut()
                .expect("created response")
                .remove("thread_reply");
            let mut expected = crate::services::nyxbot_response_recovery::Payload {
                snapshot,
                frames: vec![created],
            };
            expected.finish_text(Some(text), false);
            if payload.snapshot != expected.snapshot || payload.frames != expected.frames {
                return Err(AppError::Internal("Response notice changed".into()));
            }
        }
        view.payload = payload;
    }
    Ok(Some(view))
}

pub(super) async fn closed_post(
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
    failure: ProviderChannelFailure,
) -> Response {
    let Some(row) = &failure.closed else {
        return failure.response;
    };
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .filter(|k| !k.is_empty() && k.len() <= 256)
    else {
        return failure.response;
    };
    let Ok(request) = serde_json::from_slice::<Value>(body) else {
        return failure.response;
    };
    let id = dispatch_identity::relay_response(&row.id, key);
    let hash = sha256_hex(request.to_string());
    closed_response(state, failure, &id, Some(&hash), Format::Events(None)).await
}

pub(super) enum Format {
    Snapshot,
    Events(Option<u64>),
}

pub(super) async fn closed_response(
    state: &AppState,
    failure: ProviderChannelFailure,
    id: &str,
    request_hash: Option<&str>,
    format: Format,
) -> Response {
    let Some(row) = &failure.closed else {
        return failure.response;
    };
    let view = match read::closed(&state.db, row, id, request_hash).await {
        Ok(Some(view)) => view,
        Ok(None) => return failure.response,
        Err(error) => return provider_refusal::error(error),
    };
    if let Format::Events(after) = format {
        if after.is_some_and(|n| n >= view.payload.frames.len() as u64) {
            return problem(StatusCode::BAD_REQUEST, "invalid_replay_cursor");
        }
        let frames = view
            .payload
            .frames
            .into_iter()
            .skip(after.map_or(0, |n| n as usize + 1));
        let mut response = response_sse(futures::stream::iter(frames), id);
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().expect("static header"));
        response
    } else {
        ([("cache-control", "no-store")], Json(view.payload.snapshot)).into_response()
    }
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
            let live = provider_channel_classified(&state, &headers, false).await;
            let view = match live {
                Ok((live, _)) if live.id == row.id && live.binding_id == row.binding_id =>
                    load(&state, &live, &id).await,
                Err(failure) if failure.closed.as_ref().is_some_and(|live|live.id == row.id && live.binding_id == row.binding_id) =>
                    read::closed(&state.db,&row,&id,None).await,
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
    text: InboundReply,
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
        context_version: None,
        ciphertext: payload.seal(&state.encryption_keys).await?,
    };
    let reply = bson::to_bson(&reply)
        .map_err(|_| AppError::Internal("Response reply unavailable".into()))?;
    let mut fields = doc! { "response_reply": reply, "response_read.thread_reply":thread_reply };
    fields.insert("status", "refused");
    record_reply(&state.db, row, &event, &id, &text, fields).await?;
    Ok(response_sse(futures::stream::iter(payload.frames), &id))
}

pub(super) use read::SelectedThread;

pub(super) async fn record_reply(
    db: &mongodb::Database,
    row: &NyxbotChannel,
    event: &str,
    id: &str,
    reply: &InboundReply,
    fields: bson::Document,
) -> AppResult<()> {
    let notice = reply
        .kind
        .zip(reply.followed.as_deref())
        .map(|(kind, selected)| (kind, selected, reply.text.as_str()));
    read::record_reply(db, row, event, id, notice, fields).await
}
