use super::super::{
    openai, tokens,
    transcript::{Segment, Speaker},
};
use super::{arbiter::Arbiter, protocol};
use crate::services::billing::{BillingRouteContext, meter::MeteredProxyContext};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_voice_session::VoiceSession,
};
use base64::Engine;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};
use tokio::sync::mpsc;

pub enum ClientInput {
    Pcm(Vec<u8>),
    Begin,
    Commit,
    Playback { ms: i64, audible: bool },
}
pub enum Output {
    Audio { bytes: Vec<u8>, end_ms: i64 },
    Flush,
}
struct Input {
    id: String,
    start: i64,
    end: i64,
    samples: usize,
    voiced: usize,
    echo_samples: usize,
    overlap: bool,
}
struct Speech {
    id: String,
    item: String,
    text: String,
    start: i64,
    end: i64,
}
pub struct Grok {
    pub(super) socket: openai::Socket,
    pub(super) input: mpsc::Receiver<ClientInput>,
    output: mpsc::Sender<Output>,
    state: AppState,
    call: VoiceSession,
    billing: BillingRouteContext,
    pub(super) clock: tokio::time::Instant,
    pub arbiter: Arbiter,
    current_input: Option<Input>,
    commits: VecDeque<Input>,
    inputs: HashMap<String, Input>,
    utterances: HashMap<String, i64>,
    transcript_done: HashSet<String>,
    function_done: HashSet<String>,
    response_done: HashSet<String>,
    speech: Option<Speech>,
    last_playback: Option<(String, i64, i64)>,
    pub(super) closed_ms: Arc<AtomicI64>,
    pub(super) events: VecDeque<Value>,
    meter: Option<MeteredProxyContext>,
    sequence: u64,
    muted: bool,
    evidence: HashMap<String, Input>,
    echo: super::gate::EchoReference,
}
impl Grok {
    pub fn new(
        socket: openai::Socket,
        input: mpsc::Receiver<ClientInput>,
        output: mpsc::Sender<Output>,
        state: AppState,
        call: VoiceSession,
        billing: BillingRouteContext,
    ) -> Self {
        Self {
            socket,
            input,
            output,
            state,
            call,
            billing,
            clock: tokio::time::Instant::now(),
            arbiter: Arbiter::default(),
            current_input: None,
            commits: VecDeque::new(),
            inputs: HashMap::new(),
            utterances: HashMap::new(),
            transcript_done: HashSet::new(),
            function_done: HashSet::new(),
            response_done: HashSet::new(),
            speech: None,
            last_playback: None,
            closed_ms: Arc::new(AtomicI64::new(-1)),
            events: VecDeque::new(),
            meter: None,
            sequence: 0,
            muted: false,
            evidence: HashMap::new(),
            echo: Default::default(),
        }
    }
    pub fn started_at(mut self, clock: tokio::time::Instant) -> Self {
        self.clock = clock;
        self
    }
    pub fn measured_ms(&self) -> i64 {
        let closed = self.closed_ms.load(Ordering::Acquire);
        if closed >= 0 {
            closed
        } else {
            self.clock.elapsed().as_millis().min(1_800_000) as i64
        }
    }
    fn now(&self) -> i64 {
        (chrono::Utc::now() - self.call.created_at)
            .num_milliseconds()
            .max(0)
    }
    pub(super) async fn pump(&mut self) -> AppResult<()> {
        if let Some(line) = self.arbiter.take_next() {
            let current = super::super::session::refresh(&self.state.db, &self.call).await?;
            if current.end_requested || chrono::Utc::now() >= current.deadline {
                return Err(AppError::ClientDisconnected);
            }
            self.sequence += 1;
            self.meter = Some(
                tokens::reserve(&self.state, &self.billing, &self.call.id, self.sequence).await?,
            );
            let event = match line {
                Some(text) => {
                    json!({"type":"conversation.item.create","item":{"type":"force_message","role":"assistant",
                    "interruptible":true,"content":[{"type":"output_text","text":text}]}})
                }
                None => json!({"type":"response.create"}),
            };
            openai::send(&mut self.socket, event).await?;
        }
        Ok(())
    }
    pub async fn send(&mut self, event: Value) -> AppResult<()> {
        match event["type"].as_str() {
            Some("session.commentary.append") => self
                .arbiter
                .line(event["content"].as_str().unwrap_or_default().into())?,
            Some("nyx.readback") => self
                .arbiter
                .line(event["question"].as_str().unwrap_or_default().into())?,
            Some("nyx.input.accepted") => {
                let id = event["id"]
                    .as_str()
                    .ok_or(AppError::VoiceProviderUnavailable)?;
                if !self.utterances.contains_key(id) {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                openai::send(&mut self.socket,json!({"type":"conversation.item.create","item":{"type":"message","role":"system",
                    "content":[{"type":"input_text","text":format!("NyxID utterance_id for the preceding user's input: {id}. Use only this ID when delegating that utterance.")}]}})).await?;
                self.arbiter.respond();
            }
            Some("nyx.task_receipt") => {
                let id = event["call_id"]
                    .as_str()
                    .ok_or(AppError::VoiceProviderUnavailable)?;
                self.receipt(id, event["receipt"].clone()).await?;
            }
            Some("session.input_audio.mute" | "session.input_audio.unmute") => {
                self.muted = event["type"] == "session.input_audio.mute";
                self.current_input = None;
                openai::send(&mut self.socket, json!({"type":"input_audio_buffer.clear"})).await?;
                self.events.push_back(json!({"type":if self.muted {"session.input_audio.muted"} else {"session.input_audio.unmuted"},
                    "client_event_id":event["event_id"]}));
            }
            _ => return Err(AppError::VoiceProviderUnavailable),
        }
        self.pump().await
    }
    async fn receipt(&mut self, id: &str, receipt: Value) -> AppResult<()> {
        if self.arbiter.unresolved.remove(id) {
            openai::send(
                &mut self.socket,
                json!({"type":"conversation.item.create","item":{
                "type":"function_call_output","call_id":id,"output":receipt.to_string()}}),
            )
            .await?;
            self.arbiter.respond();
        }
        Ok(())
    }
}

impl Grok {
    pub(super) async fn client(&mut self, input: ClientInput) -> AppResult<()> {
        match input {
            ClientInput::Begin if !self.muted => {
                if self.current_input.is_some()
                    || !self.commits.is_empty()
                    || self.inputs.len() >= 4
                {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                self.current_input = Some(Input {
                    id: uuid::Uuid::new_v4().to_string(),
                    start: self.now(),
                    end: self.now(),
                    samples: 0,
                    voiced: 0,
                    echo_samples: 0,
                    overlap: self.arbiter.active.is_some()
                        || self.arbiter.playback_ms < self.arbiter.playback_end_ms,
                });
            }
            ClientInput::Pcm(bytes) if !self.muted => {
                let now = self.now();
                let input = self
                    .current_input
                    .as_mut()
                    .ok_or(AppError::VoiceProviderUnavailable)?;
                if bytes.is_empty() || bytes.len() > 1920 || bytes.len() % 2 != 0 {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                input.samples += bytes.len() / 2;
                if self.echo.matches(&bytes) {
                    input.echo_samples += bytes.len() / 2;
                }
                input.voiced += bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .filter(|b| i16::from_le_bytes(**b).unsigned_abs() > 512)
                    .count();
                input.end = now;
                if input.samples > 24_000 * 15
                    || now - input.start > 16_000
                    || input.samples as i64 > (now - input.start + 1000) * 24
                {
                    return Err(AppError::RateLimited);
                }
                openai::send(&mut self.socket,json!({"type":"input_audio_buffer.append","audio":base64::engine::general_purpose::STANDARD.encode(bytes)})).await?;
            }
            ClientInput::Commit => {
                if let Some(input) = self.current_input.take() {
                    if input.samples < 2400 || input.voiced < 240 {
                        openai::send(&mut self.socket, json!({"type":"input_audio_buffer.clear"}))
                            .await?;
                    } else {
                        self.commits.push_back(input);
                        openai::send(
                            &mut self.socket,
                            json!({"type":"input_audio_buffer.commit"}),
                        )
                        .await?;
                    }
                }
            }
            ClientInput::Playback { ms, audible } => {
                // Watermark is bounded independently by session::heartbeat. Muting
                // can drain output for scheduling but never arm a confirmation.
                if ms <= self.now() {
                    self.arbiter.playback_ms = self.arbiter.playback_ms.max(ms);
                }
                if !audible && self.arbiter.active.is_none() {
                    self.arbiter.playback_ms = self.arbiter.playback_end_ms;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn segment(&mut self, segment: Segment) -> AppResult<()> {
        if self.events.len() >= 64 {
            return Err(AppError::VoiceProviderUnavailable);
        }
        self.events
            .push_back(json!({"type":"nyx.transcript","segment":segment}));
        Ok(())
    }
    pub(super) async fn provider(&mut self, event: Value) -> AppResult<()> {
        match event["type"].as_str().unwrap_or_default() {
            "input_audio_buffer.committed" => {
                let id = field(&event, "item_id")?;
                if self.inputs.contains_key(id) {
                    return Ok(());
                }
                let input = self
                    .commits
                    .pop_front()
                    .ok_or(AppError::VoiceProviderUnavailable)?;
                self.inputs.insert(id.into(), input);
            }
            "conversation.item.input_audio_transcription.updated"
            | "conversation.item.input_audio_transcription.completed" => {
                let item = field(&event, "item_id")?;
                if self.transcript_done.contains(item) {
                    return Ok(());
                }
                let Some(input) = self.inputs.get(item) else {
                    return Ok(());
                };
                let complete =
                    event["type"] == "conversation.item.input_audio_transcription.completed";
                let text = event["transcript"]
                    .as_str()
                    .filter(|s| s.len() <= 4096)
                    .ok_or(AppError::VoiceProviderUnavailable)?;
                let segment = Segment {
                    id: input.id.clone(),
                    speaker: Speaker::User,
                    text: text.into(),
                    start_ms: input.start,
                    end_ms: input.end,
                    sealed: complete,
                    complete,
                };
                if complete {
                    let cutoff = self.now() - 60_000;
                    self.utterances.retain(|_, end| *end >= cutoff);
                    self.evidence.retain(|_, input| input.end >= cutoff);
                    let input = self.inputs.get(item).expect("known input");
                    self.utterances.insert(input.id.clone(), input.end);
                    if let Some(input) = self.inputs.remove(item) {
                        self.evidence.insert(input.id.clone(), input);
                    }
                    self.transcript_done.insert(item.into());
                    if self.utterances.len() > 256 {
                        return Err(AppError::VoiceProviderUnavailable);
                    }
                }
                self.segment(segment)?;
            }
            "response.created" => {
                let id = field(&event["response"], "id")?;
                self.arbiter.created(id.into())?;
                self.speech = None;
            }
            "response.function_call_arguments.done" => {
                let id = field(&event, "call_id")?.to_owned();
                if !self.function_done.insert(id.clone()) {
                    return Ok(());
                }
                if self.function_done.len() > 256
                    || self.arbiter.active.as_deref() != Some(field(&event, "response_id")?)
                {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                self.arbiter.unresolved.insert(id.clone());
                if let Some(args) = protocol::delegation(&event)
                    && let Some(end) = self.utterances.get(&args.utterance_id)
                {
                    self.events.push_back(json!({"type":"session.delegation.created","delegation":{"id":id,"target":"client"},
                        "offset_ms":end,"utterance_id":args.utterance_id}));
                } else {
                    self.receipt(
                        &id,
                        json!({"status":"refused","reason":"invalid_utterance_or_function"}),
                    )
                    .await?;
                }
            }
            "error" => return Err(AppError::VoiceProviderUnavailable),
            _ => self.response_event(event).await?,
        }
        Ok(())
    }
}
fn field<'a>(event: &'a Value, name: &str) -> AppResult<&'a str> {
    event[name]
        .as_str()
        .filter(|s| openai::valid_provider_id(s))
        .ok_or(AppError::VoiceProviderUnavailable)
}

impl Grok {
    async fn response_event(&mut self, event: Value) -> AppResult<()> {
        match event["type"].as_str().unwrap_or_default() {
            "response.output_audio.delta" | "response.audio.delta" => {
                let response = field(&event, "response_id")?;
                if self.arbiter.cancelled.contains(response) {
                    return Ok(());
                }
                if self.arbiter.active.as_deref() != Some(response) {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                let item = field(&event, "item_id")?.to_owned();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(
                        event["delta"]
                            .as_str()
                            .ok_or(AppError::VoiceProviderUnavailable)?,
                    )
                    .map_err(|_| AppError::VoiceProviderUnavailable)?;
                if bytes.is_empty() || bytes.len() % 2 != 0 || bytes.len() > 192_000 {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                self.echo.output(&bytes);
                let now = self.now();
                let speech = self.speech.get_or_insert(Speech {
                    id: uuid::Uuid::new_v4().to_string(),
                    item,
                    text: String::new(),
                    start: now,
                    end: now,
                });
                if speech.item != event["item_id"].as_str().unwrap_or_default() {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                for chunk in bytes.chunks(1920) {
                    speech.end = speech.end.max(now) + (chunk.len() as i64 / 48);
                    if speech.end - now > 5000 {
                        return Err(AppError::VoiceProviderUnavailable);
                    }
                    tokio::time::timeout(
                        std::time::Duration::from_secs(1),
                        self.output.send(Output::Audio {
                            bytes: chunk.to_vec(),
                            end_ms: speech.end,
                        }),
                    )
                    .await
                    .map_err(|_| AppError::ClientDisconnected)?
                    .map_err(|_| AppError::ClientDisconnected)?;
                }
                self.arbiter.playback_end_ms = speech.end;
                self.last_playback = Some((speech.item.clone(), speech.start, speech.end));
            }
            "response.output_audio_transcript.delta"
            | "response.audio_transcript.delta"
            | "response.output_audio_transcript.done"
            | "response.audio_transcript.done" => {
                let response = field(&event, "response_id")?;
                if self.arbiter.cancelled.contains(response) {
                    return Ok(());
                }
                if self.arbiter.active.as_deref() != Some(response) {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                let now = self.now();
                let item = field(&event, "item_id")?.to_owned();
                let speech = self.speech.get_or_insert(Speech {
                    id: uuid::Uuid::new_v4().to_string(),
                    item,
                    text: String::new(),
                    start: now,
                    end: now,
                });
                if speech.item != event["item_id"].as_str().unwrap_or_default() {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                if let Some(text) = event["transcript"].as_str() {
                    speech.text = text.into();
                } else if let Some(text) = event["delta"].as_str() {
                    speech.text.push_str(text);
                }
                if speech.text.len() > 8000 {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                let segment = Segment {
                    id: speech.id.clone(),
                    speaker: Speaker::Assistant,
                    text: speech.text.clone(),
                    start_ms: speech.start,
                    end_ms: speech.end,
                    sealed: false,
                    complete: false,
                };
                self.segment(segment)?;
            }
            "response.done" => {
                let id = field(&event["response"], "id")?;
                if !self.response_done.insert(id.into()) {
                    return Ok(());
                }
                if self.arbiter.active.as_deref() != Some(id) {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                let meter = self
                    .meter
                    .take()
                    .ok_or(AppError::VoiceProviderUnavailable)?;
                tokens::settle(&self.state, &meter, &event, &self.call.preferences.model).await?;
                if let Some(speech) = self.speech.take() {
                    self.segment(Segment {
                        id: speech.id,
                        speaker: Speaker::Assistant,
                        text: speech.text,
                        start_ms: speech.start,
                        end_ms: speech.end,
                        sealed: true,
                        complete: !self.arbiter.cancelled.contains(id)
                            && event["response"]["status"] == "completed"
                            && speech.end > speech.start,
                    })?;
                }
                self.arbiter.done(id)?;
                if self.response_done.len() > 512 {
                    return Err(AppError::VoiceProviderUnavailable);
                }
            }
            _ => {} // VAD/model claims/unknown frames never cancel or approve work.
        }
        Ok(())
    }
    pub async fn close(&mut self) -> AppResult<()> {
        if self.closed_ms.load(Ordering::Acquire) >= 0 {
            return Ok(());
        }
        // Closing this server-owned transport also ends the sole provider media path.
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(2), self.socket.close(None)).await;
        self.closed_ms.store(self.measured_ms(), Ordering::Release);
        // No duration is charged for subsequent database settlement work.
        // Unreported tokens are zero; unknown provider cost is platform exposure.
        if let Some(meter) = self.meter.take() {
            self.state
                .billing
                .settle_deferred(
                    &meter,
                    Default::default(),
                    None,
                    Some(self.call.preferences.model.clone()),
                )
                .await?;
        }
        result
            .map_err(|_| AppError::VoiceProviderUnavailable)?
            .map_err(|_| AppError::VoiceProviderUnavailable)
    }
}

impl Grok {
    pub async fn accept_input(&mut self, segment: &Segment) -> AppResult<bool> {
        let state = self.state.clone();
        let actor = self.call.user_id.clone();
        self.accept_input_with(
            segment,
            &super::gate::OneShot {
                state: &state,
                actor: &actor,
            },
        )
        .await
    }
    pub(super) async fn accept_input_with(
        &mut self,
        segment: &Segment,
        classifier: &dyn super::gate::Classifier,
    ) -> AppResult<bool> {
        let Some(input) = self.evidence.remove(&segment.id) else {
            return Ok(false);
        };
        if !input.overlap {
            return Ok(true);
        }
        if input.samples < 7200 || input.voiced < 240 || input.echo_samples * 2 >= input.samples {
            return Ok(false);
        }
        if !classifier.meaningful(&segment.text).await {
            return Ok(false);
        }
        // Explicit PTT plus voiced, non-correlated authoritative transcript evidence.
        // Never cancel on speech_started/VAD, and never cancel backend work here.
        if let Some(id) = &self.arbiter.active {
            self.arbiter.cancelled.insert(id.clone());
            openai::send(
                &mut self.socket,
                json!({"type":"response.cancel","response_id":id}),
            )
            .await?;
        }
        if let Some((item, start, end)) = &self.last_playback {
            let played = (self.arbiter.playback_ms.min(*end) - start).max(0);
            openai::send(
                &mut self.socket,
                json!({"type":"conversation.item.truncate","item_id":item,
                "content_index":0,"audio_end_ms":played}),
            )
            .await?;
        }
        self.arbiter.playback_end_ms = self.arbiter.playback_ms;
        self.output
            .try_send(Output::Flush)
            .map_err(|_| AppError::ClientDisconnected)?;
        Ok(true)
    }
}
