//! Bounded composer transcription. No tools, assistant responses, audio storage or replay.
use super::{credentials, openai, runtime, session};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_voice::VoicePreferences,
        assistant_voice_session::{COLLECTION_NAME, VoiceSession},
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

pub const MAX_SECONDS: i64 = 120;
const BYTES_PER_SECOND: usize = 48_000;
const COMMIT_BYTES: usize = 5 * BYTES_PER_SECOND;
const PICO: i64 = 1_000_000_000_000;
pub enum Input {
    Audio(Vec<u8>),
    Stop,
}
pub type Output = Value;

/// Platform only for the first transport. Own-key operation selection is deliberately
/// not synthesized from a conversational source. Existing rollout, tariff and ACL apply.
pub async fn resolve(
    state: &AppState,
    user: &str,
    thread: &str,
    p: &VoicePreferences,
) -> AppResult<credentials::Resolved> {
    super::super::assistant_voice::require_enabled(&state.db, user).await?;
    if p.key_source != crate::models::assistant_voice::VoiceKeySource::Platform
        || p.connection_id.is_some()
    {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let resolved = Box::pin(credentials::resolve_audio(state, user, thread, p, true)).await?;
    if resolved.protocol != crate::models::downstream_service::VoiceProtocol::OpenaiLive {
        return Err(AppError::VoiceProviderUnavailable);
    }
    Ok(resolved)
}

pub fn configuration() -> Value {
    json!({"type":"session.update","session":{"type":"transcription","audio":{"input":{
        "format":{"type":"audio/pcm","rate":24000},"transcription":{"model":"gpt-live-transcribe"},"turn_detection":null
    }}}})
}
enum Provider {
    Production,
    #[cfg(test)]
    Fixture(Box<openai::Socket>),
}
fn valid_configuration(event: &Value) -> bool {
    let s = &event["session"];
    matches!(
        event["type"].as_str(),
        Some("session.updated" | "transcription_session.updated")
    ) && s["type"] == "transcription"
        && s["audio"]["input"]["transcription"]["model"] == "gpt-live-transcribe"
        && s["audio"]["input"]["format"]["type"] == "audio/pcm"
        && s["audio"]["input"]["format"]["rate"] == 24000
        && s["audio"]["input"]["turn_detection"].is_null()
        && s.get("tools")
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty))
}
async fn connect(key: &str, provider: Provider) -> AppResult<openai::Socket> {
    let mut socket = match provider {
        #[cfg(test)]
        Provider::Fixture(socket) => *socket,
        Provider::Production => connect_official(key).await?,
    };
    openai::send(&mut socket, configuration()).await?;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = openai::receive(&mut socket)
                .await?
                .ok_or(AppError::VoiceProviderUnavailable)?;
            match event["type"].as_str() {
                Some("session.updated" | "transcription_session.updated") => {
                    return if valid_configuration(&event) {
                        Ok(())
                    } else {
                        Err(AppError::VoiceProviderUnavailable)
                    };
                }
                Some("error") => return Err(AppError::VoiceProviderUnavailable),
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| AppError::VoiceProviderUnavailable)??;
    Ok(socket)
}
async fn connect_official(key: &str) -> AppResult<openai::Socket> {
    let mut request = "wss://api.openai.com/v1/realtime?intent=transcription"
        .into_client_request()
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    let mut auth = format!("Bearer {key}")
        .parse::<http::HeaderValue>()
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    auth.set_sensitive(true);
    request
        .headers_mut()
        .insert(http::header::AUTHORIZATION, auth);
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(openai::MAX_FRAME))
        .max_frame_size(Some(openai::MAX_FRAME));
    let (socket, _) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_tungstenite::connect_async_with_config(request, Some(config), false),
    )
    .await
    .map_err(|_| AppError::VoiceProviderUnavailable)?
    .map_err(|_| AppError::VoiceProviderUnavailable)?;
    Ok(socket)
}

fn item_id(event: &Value) -> AppResult<&str> {
    event["item_id"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 256
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        .ok_or(AppError::VoiceProviderUnavailable)
}
/// Exact decimal carry in picoseconds; never round each provider item to seconds.
/// Unknown/token usage shapes fail closed for the fixed duration-billed model.
pub fn confirmed_duration(event: &Value) -> AppResult<i64> {
    if event["usage"]["type"] != "duration" || !event["usage"]["seconds"].is_number() {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let raw = event["usage"]["seconds"].to_string();
    let seconds = raw
        .parse::<crate::models::credits::Credits>()
        .ok()
        .and_then(|value| i64::try_from(value.pico()).ok())
        .ok_or(AppError::VoiceProviderUnavailable)?;
    if !(0..=MAX_SECONDS * PICO).contains(&seconds) {
        return Err(AppError::VoiceProviderUnavailable);
    }
    Ok(seconds)
}

/// Item dedup and cumulative confirmed usage commit together under the live lease.
/// Recovery settles from observed_seconds even if a process dies before window writes
/// or the browser acknowledgement. No transcript text is stored here.
pub async fn checkpoint(
    state: &AppState,
    call: &VoiceSession,
    item: &str,
    duration: i64,
) -> AppResult<VoiceSession> {
    let db = state.db.clone();
    let call = call.clone();
    let item = item.to_string();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction().and_run2(async move |tx| {
        let result: AppResult<VoiceSession> = Box::pin(async {
            let rows = db.collection::<bson::Document>(COLLECTION_NAME);
            let mut checkpoint_fence = session::fence(&call);
            checkpoint_fence.insert("state", doc!{"$in":["starting","active"]});
            let row = rows.find_one(checkpoint_fence.clone()).session(&mut *tx).await?.ok_or_else(||AppError::Conflict("Dictation lease lost".into()))?;
            let previous = match row.get("dictation_duration_pico") { None => 0, Some(bson::Bson::Int64(v)) if *v >= 0 => *v, _ => return Err(AppError::VoiceProviderUnavailable) };
            let items = match row.get("dictation_items") { None => Vec::new(), Some(bson::Bson::Array(v)) => v.clone(), _ => return Err(AppError::VoiceProviderUnavailable) };
            if items.iter().any(|v| v.as_document().is_none_or(|d| d.get_str("id").is_err() || d.get_i64("duration").is_err())) { return Err(AppError::VoiceProviderUnavailable); }
            if let Some(saved) = items.iter().filter_map(bson::Bson::as_document).find(|v|v.get_str("id").ok()==Some(&item)) {
                if saved.get_i64("duration").ok()!=Some(duration) { return Err(AppError::VoiceProviderUnavailable); }
                return bson::from_document(row).map_err(|_|AppError::VoiceProviderUnavailable);
            }
            if items.len() >= 128 || duration < 0 {return Err(AppError::VoiceProviderUnavailable);}
            let total = previous.checked_add(duration).filter(|v|*v <= MAX_SECONDS*PICO).ok_or(AppError::VoiceProviderUnavailable)?;
            let reserved = match row.get("reserved_until") {
                Some(bson::Bson::Int32(v)) => i64::from(*v),
                Some(bson::Bson::Int64(v)) => *v,
                _ => return Err(AppError::VoiceProviderUnavailable),
            };
            if !(0..=MAX_SECONDS).contains(&reserved) {return Err(AppError::VoiceProviderUnavailable);}
            if total > reserved*PICO {return Err(AppError::VoiceProviderUnavailable);}
            let updated = rows.find_one_and_update(checkpoint_fence,doc!{
                "$set":{"dictation_duration_pico":total,"observed_seconds":total/PICO,"usage_source":"provider_transcription_duration"},
                "$push":{"dictation_items":{"id":&item,"duration":duration}}
            }).return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?.ok_or(AppError::VoiceProviderUnavailable)?;
            bson::from_document(updated).map_err(|_|AppError::VoiceProviderUnavailable)
        }).await;
        super::super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
}

pub async fn finish(state: &AppState, call: &VoiceSession, confirmed: bool) -> AppResult<()> {
    // Freeze confirmed usage under the current generation before sealing money.
    // The returned row, not the caller's snapshot, is the settlement authority.
    let closing = session::write(&state.db, call, doc!{"state":"closing","end_requested":true,
        "lease_until":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::seconds(15))}).await?;
    runtime::settle_windows(state, &closing, !confirmed).await?;
    session::write(&state.db, &closing, doc!{"state":"closed","live_slot":false,"end_requested":true,
        "closed_at":bson::DateTime::now(),"billing_finalized":true,"final_usage_confirmed":confirmed,
        "end_reason":if confirmed {"dictation_complete"} else {"dictation_unconfirmed"}}).await?;
    Ok(())
}

async fn emit(output: &mpsc::Sender<Output>, event: Value) -> AppResult<()> {
    // Sink loss is UI cancellation, not permission to skip provider usage drain.
    // A bounded channel prevents transcript backpressure from retaining raw audio.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), output.send(event)).await;
    Ok(())
}
struct Drain(Option<tokio::time::Instant>);
impl Drain {
    fn stop(&mut self) {
        self.0.get_or_insert_with(tokio::time::Instant::now);
    }
    fn stopping(&self) -> bool {
        self.0.is_some()
    }
    fn expired(&self) -> bool {
        self.0.is_some_and(|at| at.elapsed().as_secs() > 12)
    }
}

// Keep the authenticated actor/thread/session and the two bounded channel ends
// explicit at this relay boundary, matching serve_with_provider below.
#[allow(clippy::too_many_arguments)]
pub async fn serve(
    state: AppState,
    user: String,
    thread: String,
    human_session: String,
    request: String,
    preferences: VoicePreferences,
    input: mpsc::Receiver<Input>,
    output: mpsc::Sender<Output>,
) {
    serve_with_provider(
        state,
        user,
        thread,
        human_session,
        request,
        preferences,
        input,
        output,
        Provider::Production,
    )
    .await;
}
#[allow(clippy::too_many_arguments)]
async fn serve_with_provider(
    state: AppState,
    user: String,
    thread: String,
    human_session: String,
    request: String,
    preferences: VoicePreferences,
    mut input: mpsc::Receiver<Input>,
    output: mpsc::Sender<Output>,
    injected: Provider,
) {
    let mut admitted = None;
    let result: AppResult<()> = Box::pin(async {
        let resolved = resolve(&state, &user, &thread, &preferences).await?;
        let concurrency = super::super::service_concurrency_service::acquire_policy(&state.db, resolved.concurrency_policy.as_ref(), &user).await?;
        let mut call = session::admit_dictation(&state.db,&user,&thread,&request,preferences,resolved.identity.clone(),&state.replica_identity.generation_id).await?;
        admitted = Some(call.clone());
        let meter = super::super::billing::voice::reserve(&state.db,&state.billing,resolved.billing.clone(),&call.id,0).await?;
        call = session::write(&state.db,&call,doc!{"reserved_until":30,"deadline":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::seconds(MAX_SECONDS+15))}).await?;
        admitted = Some(call.clone());
        // Recheck the browser credential after admission/funding, before provider effects.
        if state.db.collection::<bson::Document>(crate::models::session::COLLECTION_NAME)
            .find_one(doc!{"_id":&human_session,"user_id":&user,"revoked":false,"expires_at":{"$gt":bson::DateTime::now()}}).await?.is_none() {
            return Err(AppError::Forbidden("Browser session expired".into()));
        }
        let fresh = resolve(&state,&user,&thread,&call.preferences).await?;
        if fresh.identity != call.credential_identity { return Err(AppError::VoiceProviderUnavailable); }
        state.billing.mark_forwarded(&meter).await?;
        let mut provider = connect(&resolved.key, injected).await?;
        call = session::write(&state.db,&call,doc!{"state":"active"}).await?;
        admitted = Some(call.clone());
        emit(&output,json!({"type":"ready","session_id":call.id,"model":"gpt-live-transcribe"})).await?;
        let started = tokio::time::Instant::now();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
        let mut forwarded = 0usize;
        let mut buffered = 0usize;
        let mut pending = false;
        let mut pending_item: Option<String> = None;
        let mut pending_at = tokio::time::Instant::now();
        let mut drain = Drain(None);
        let mut orders = std::collections::HashMap::<String, usize>::new();
        let mut committed_count = 0usize;
        let mut completed = std::collections::HashSet::new();
        let mut text_bytes = 0usize;
        loop {
            tokio::select! { biased;
                _ = tick.tick() => {
                    if started.elapsed().as_secs() >= MAX_SECONDS as u64 {drain.stop();}
                    if pending && pending_at.elapsed().as_secs()>10 {return Err(AppError::VoiceProviderUnavailable);}
                    let active = state.db.collection::<bson::Document>(crate::models::session::COLLECTION_NAME)
                        .find_one(doc!{"_id":&human_session,"user_id":&user,"revoked":false,"expires_at":{"$gt":bson::DateTime::now()}}).await?.is_some();
                    if !active {return Err(AppError::Forbidden("Browser session expired".into()));}
                    let fresh=resolve(&state,&user,&thread,&call.preferences).await?;
                    if fresh.identity!=call.credential_identity {return Err(AppError::VoiceProviderUnavailable);}
                    call=session::refresh(&state.db,&call).await?;
                    if call.end_requested {drain.stop();}
                    admitted=Some(call.clone());
                }
                () = async { if let Some(lease)=&concurrency {lease.cancelled().await} else {std::future::pending().await} } => return Err(AppError::ServiceConcurrencyLimited),
                next = input.recv(), if !drain.stopping() => {
                    match next {
                        Some(Input::Audio(bytes)) => {
                            if bytes.is_empty() || bytes.len()>4096 || bytes.len()%2!=0 || forwarded+bytes.len()>MAX_SECONDS as usize*BYTES_PER_SECOND
                                || forwarded+bytes.len() > call.reserved_until as usize*BYTES_PER_SECOND
                                || forwarded+bytes.len() > ((started.elapsed().as_secs_f64()+1.0)*BYTES_PER_SECOND as f64) as usize {
                                return Err(AppError::ValidationError("Dictation audio limit exceeded".into()));
                            }
                            // Revalidate before each new funded window. Never spend beyond it.
                            if forwarded+bytes.len() > (call.reserved_until-10).max(0) as usize*BYTES_PER_SECOND && call.reserved_until<MAX_SECONDS {
                                let fresh = resolve(&state,&user,&thread,&call.preferences).await?;
                                if fresh.identity!=call.credential_identity {return Err(AppError::VoiceProviderUnavailable);}
                                let meter = super::super::billing::voice::reserve(&state.db,&state.billing,fresh.billing,&call.id,call.reserved_until).await?;
                                state.billing.mark_forwarded(&meter).await?;
                                call=session::write(&state.db,&call,doc!{"reserved_until":call.reserved_until+30}).await?;
                                admitted=Some(call.clone());
                            }
                            openai::send(&mut provider,json!({"type":"input_audio_buffer.append","audio":STANDARD.encode(&bytes)})).await?;
                            forwarded+=bytes.len(); buffered+=bytes.len();
                        }
                        Some(Input::Stop) | None => {drain.stop();}
                    }
                }
                event = openai::receive(&mut provider) => {
                    let event = event?.ok_or(AppError::VoiceProviderUnavailable)?;
                    match event["type"].as_str() {
                        Some("input_audio_buffer.committed") => {
                            if !pending || pending_item.is_some() { return Err(AppError::VoiceProviderUnavailable); }
                            let id = item_id(&event)?.to_owned();
                            let order = *orders.entry(id.clone()).or_insert(committed_count);
                            if order != committed_count {return Err(AppError::VoiceProviderUnavailable);}
                            committed_count += 1;
                            pending_item = Some(id);
                        }
                        Some("conversation.item.input_audio_transcription.delta") => {
                            let id = item_id(&event)?;
                            if !completed.contains(id) {
                                let text = event["delta"].as_str().filter(|s|s.len()<=4096).ok_or(AppError::VoiceProviderUnavailable)?;
                                text_bytes += text.len();
                                if text_bytes > 128_000 {return Err(AppError::VoiceProviderUnavailable);}
                                if !orders.contains_key(id) && orders.values().any(|order| *order == committed_count) {
                                    return Err(AppError::VoiceProviderUnavailable);
                                }
                                let order = *orders.entry(id.to_owned()).or_insert(committed_count);
                                emit(&output,json!({"type":"delta","item_id":id,"order":order,"text":text})).await?;
                            }
                        }
                        Some("conversation.item.input_audio_transcription.completed") => {
                            let id = item_id(&event)?;
                            let duration = confirmed_duration(&event)?;
                            if !completed.contains(id) && (!pending || pending_item.as_deref()!=Some(id)) { return Err(AppError::VoiceProviderUnavailable); }
                            call = checkpoint(&state,&call,id,duration).await?;
                            admitted = Some(call.clone());
                            if completed.insert(id.to_owned()) {
                                let text = event["transcript"].as_str().filter(|s|s.len()<=32_000).ok_or(AppError::VoiceProviderUnavailable)?;
                                pending=false; pending_item=None;
                                let order = orders.get(id).ok_or(AppError::VoiceProviderUnavailable)?;
                                emit(&output,json!({"type":"completed","item_id":id,"order":order,"text":text,"confirmed_seconds":call.observed_seconds})).await?;
                            }
                        }
                        Some("error" | "conversation.item.input_audio_transcription.failed") => return Err(AppError::VoiceProviderUnavailable),
                        _ => {}
                    }
                }


            }
            if orders.len() > 128 { return Err(AppError::VoiceProviderUnavailable); }
            if output.is_closed() { drain.stop(); }
            if !pending && buffered >= BYTES_PER_SECOND/10 && (buffered>=COMMIT_BYTES || drain.stopping()) {
                openai::send(&mut provider,json!({"type":"input_audio_buffer.commit"})).await?;
                pending=true; pending_at=tokio::time::Instant::now(); buffered=0;
            }
            if drain.stopping() && !pending {
                // Less than the provider minimum commit buffer is never billed from a clock.
                let confirmed=buffered==0;
                let _=tokio::time::timeout(std::time::Duration::from_secs(2),provider.close(None)).await;
                finish(&state,&call,confirmed).await?;
                admitted=None;
                emit(&output,json!({"type":"stopped","confirmed":confirmed})).await?;
                return Ok(());
            }
            if drain.expired() {return Err(AppError::VoiceProviderUnavailable);}
        }
    }).await;
    if let Some(call) = admitted {
        let _ = finish(&state, &call, false).await;
    }
    if let Err(error) = result {
        // Existing sanitized error envelope. Never forward provider payloads or keys.
        let _ = emit(
            &output,
            json!({"type":"error","error":error.response_body()}),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dictation_duration_is_provider_confirmed_and_fractional() {
        let a = confirmed_duration(&json!({"usage":{"type":"duration","seconds":0.6}})).unwrap();
        let b = confirmed_duration(&json!({"usage":{"type":"duration","seconds":0.6}})).unwrap();
        assert_eq!((a + b) / PICO, 1);
        for usage in [
            json!(null),
            json!({"type":"tokens","total_tokens":100}),
            json!({"type":"duration","seconds":-1}),
            json!({"type":"duration","seconds":"2"}),
            json!({"type":"duration","seconds":121}),
        ] {
            assert!(confirmed_duration(&json!({"usage":usage})).is_err());
        }
    }
    #[test]
    fn dictation_configuration_cannot_generate_audio_or_tools() {
        let value = configuration();
        assert_eq!(value["session"]["type"], "transcription");
        assert_eq!(
            value["session"]["audio"]["input"]["transcription"]["model"],
            "gpt-live-transcribe"
        );
        assert!(value["session"]["audio"]["input"]["turn_detection"].is_null());
        assert!(value["session"].get("tools").is_none());
        assert!(value["session"]["audio"].get("output").is_none());
    }
}

#[cfg(test)]
#[path = "dictation_tests.rs"]
mod integration_tests;
