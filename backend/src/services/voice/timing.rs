//! Closed, bounded diagnostic DTOs. No content, upstream identities or inferred phases.
use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::TurnActivity,
        assistant_message::TurnTiming,
        assistant_voice::{VoiceExecutionTiming, VoiceRequest, VoiceToolTiming},
        assistant_voice_session::{COLLECTION_NAME, VoiceSession},
    },
};
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::{Deserialize, Serialize};

pub const MAX_OBSERVATIONS: usize = 32;
const MAX_MS: i64 = 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    InputStart,
    InputEnd,
    TranscriptObserved,
    Delegation,
    Queued,
    FirstActivity,
    ToolStart,
    ToolEnd,
    FirstProgress,
    ApprovalWait,
    Result,
    PlaybackConfirmed,
    ModelStart,
    ModelEnd,
    Model,
    Tool,
    Total,
    OutputStart,
    OutputEnd,
    ResponseStart,
    ResponseEnd,
    ResponseGeneration,
    FirstAudio,
    McpRoundTrip,
    TranscriptionTurnaround,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Coordinator,
    ProviderTranscript,
    RelayAudio,
    NativeResponses,
    Nyxagent,
    McpClient,
    ClientPlayback,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Offset,
    Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    pub phase: Phase,
    pub source: Source,
    pub kind: Kind,
    pub value_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_index: Option<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub unit: String,
    pub offset_origin: String,
    pub measurements: Vec<Measurement>,
    pub unavailable: Vec<String>,
}
impl Default for Report {
    fn default() -> Self {
        Self {
            unit: "ms".into(),
            offset_origin: "call_created_at".into(),
            measurements: Vec::new(),
            unavailable: Vec::new(),
        }
    }
}
impl Report {
    pub fn add(&mut self, phase: Phase, source: Source, kind: Kind, ms: i64) {
        if (0..=MAX_MS).contains(&ms) && self.measurements.len() < 128 {
            self.measurements.push(Measurement {
                phase,
                source,
                kind,
                value_ms: ms,
                tool_index: None,
            });
        }
    }
    pub fn finish(mut self) -> Self {
        self.unavailable = ["asr_processing", "synthesis_processing", "device_playback"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        for (name, phase) in [
            ("model", Phase::Model),
            ("tool", Phase::Tool),
            ("first_audio", Phase::FirstAudio),
            ("client_playback", Phase::PlaybackConfirmed),
        ] {
            if !self.measurements.iter().any(|m| {
                m.phase == phase || (phase == Phase::Tool && m.phase == Phase::McpRoundTrip)
            }) {
                self.unavailable.push(name.into());
            }
        }
        self
    }
}

/// Existing request marks are offsets; legacy first_audio was a playback proof.
pub fn request_report(request: &VoiceRequest) -> Report {
    use {Kind::*, Phase::*, Source::*};
    let mut report = Report::default();
    for (key, phase, source, kind) in [
        ("transcription_ms", InputEnd, Coordinator, Offset),
        ("delegation_ms", Delegation, Coordinator, Offset),
        ("queued_ms", Queued, Coordinator, Offset),
        ("first_activity_ms", FirstActivity, Nyxagent, Offset),
        ("first_progress_ms", FirstProgress, Coordinator, Offset),
        ("approval_wait_ms", ApprovalWait, Coordinator, Offset),
        ("result_ms", Result, Coordinator, Offset),
        (
            "native_model_started_ms",
            ModelStart,
            NativeResponses,
            Offset,
        ),
        (
            "native_model_finished_ms",
            ModelEnd,
            NativeResponses,
            Offset,
        ),
        ("native_model_duration_ms", Model, NativeResponses, Duration),
    ] {
        if let Some(ms) = request.timing.get(key) {
            report.add(phase, source, kind, *ms);
        }
    }
    if let Some(ms) = request
        .timing
        .get("playback_confirmed_ms")
        .or_else(|| request.timing.get("first_audio_ms"))
    {
        report.add(PlaybackConfirmed, ClientPlayback, Offset, *ms);
    }
    if let Some(t) = request
        .execution_timing
        .as_ref()
        .filter(|t| t.turn_id == request.turn_id)
    {
        for (phase, ms) in [(Model, t.model_ms), (Tool, t.tool_ms), (Total, t.total_ms)] {
            if let Some(ms) = ms.and_then(|v| i64::try_from(v).ok()) {
                report.add(phase, Nyxagent, Duration, ms);
            }
        }
        for (index, tool) in t.tools.iter().take(40).enumerate() {
            if !(0..=MAX_MS).contains(&tool.start_ms) {
                continue;
            }
            let mut values = vec![(ToolStart, Offset, tool.start_ms)];
            if let Some(end) = tool
                .end_ms
                .filter(|end| *end >= tool.start_ms && *end <= MAX_MS)
            {
                values.push((ToolEnd, Offset, end));
                values.push((Tool, Duration, end - tool.start_ms));
            }
            for (phase, kind, ms) in values {
                let before = report.measurements.len();
                report.add(phase, Nyxagent, kind, ms);
                if report.measurements.len() > before {
                    report.measurements.last_mut().unwrap().tool_index = Some(index as u8);
                }
            }
        }
    }
    report.finish()
}

/// Only call this with the exact request's active turn or settled execution reply.
/// Generic thinking activities are not tools; a recorded MCP digest proves the interval.
pub fn execution_snapshot(
    turn_id: &str,
    start: DateTime<Utc>,
    timing: Option<&TurnTiming>,
    activities: &[TurnActivity],
) -> VoiceExecutionTiming {
    VoiceExecutionTiming {
        turn_id: turn_id.into(),
        model_ms: timing
            .filter(|t| t.model_calls > 0 && t.model_ms > 0)
            .map(|t| t.model_ms),
        tool_ms: timing
            .filter(|t| t.tool_calls > 0 && t.tool_ms > 0)
            .map(|t| t.tool_ms),
        total_ms: timing.filter(|t| t.total_ms > 0).map(|t| t.total_ms),
        tools: activities
            .iter()
            .take(40)
            .filter(|a| {
                a.digest.is_some()
                    && !a
                        .failure
                        .as_ref()
                        .is_some_and(|f| f.kind == "retry_duplicate_action")
            })
            .filter_map(|a| {
                let start_ms = (a.started_at - start).num_milliseconds();
                if !(0..=MAX_MS).contains(&start_ms) {
                    return None;
                }
                let end_ms = a
                    .ended_at
                    .filter(|end| *end >= a.started_at)
                    .map(|end| (end - start).num_milliseconds())
                    .filter(|ms| *ms <= MAX_MS);
                Some(VoiceToolTiming { start_ms, end_ms })
            })
            .collect(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_turn_id: Option<String>,
    pub kind: ObservationKind,
    pub phase_timing: Report,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Input,
    Output,
    NativeResponse,
    DirectTool,
    VoiceResponse,
}

pub fn observations(call: &VoiceSession) -> Vec<Observation> {
    call.phase_observations
        .iter()
        .rev()
        .take(MAX_OBSERVATIONS)
        .filter_map(|v| {
            let mut row = bson::from_bson::<Observation>(v.clone()).ok()?;
            if uuid::Uuid::parse_str(&row.turn_id).is_err()
                || row
                    .related_turn_id
                    .as_ref()
                    .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
                || row.phase_timing.unit != "ms"
                || row.phase_timing.offset_origin != "call_created_at"
                || row.phase_timing.measurements.len() > 128
            {
                return None;
            }
            row.phase_timing.measurements.retain(|m| {
                (0..=MAX_MS).contains(&m.value_ms) && m.tool_index.is_none_or(|i| i < 40)
            });
            // Never trust persisted arbitrary unavailable strings as diagnostic copy.
            row.phase_timing = row.phase_timing.finish();
            Some(row)
        })
        .collect()
}

/// One bounded write per completed observation or newly confirmed playback.
/// Optional telemetry cannot fail a tool/result or retain a provider payload.
pub async fn record(
    db: &Database,
    call: &VoiceSession,
    turn_id: &str,
    kind: ObservationKind,
    report: Report,
) {
    record_related(db, call, turn_id, None, kind, report).await;
}
pub async fn record_related(
    db: &Database,
    call: &VoiceSession,
    turn_id: &str,
    related_turn_id: Option<&str>,
    kind: ObservationKind,
    report: Report,
) {
    if uuid::Uuid::parse_str(turn_id).is_err() {
        return;
    }
    let observation = Observation {
        turn_id: turn_id.into(),
        related_turn_id: related_turn_id.map(str::to_owned),
        kind,
        phase_timing: report.finish(),
    };
    record_batch(db, call, vec![observation]).await;
}
async fn record_batch(db: &Database, call: &VoiceSession, observations: Vec<Observation>) {
    if observations.is_empty() {
        return;
    }
    let ids: Vec<_> = observations.iter().map(|o| o.turn_id.as_str()).collect();
    let Ok(values) = bson::to_bson(&observations) else {
        return;
    };
    let mut scope = super::session::fence(call);
    scope.extend(doc! {"user_id":&call.user_id,"conversation_id":&call.conversation_id});
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        db.collection::<bson::Document>(COLLECTION_NAME).update_one(scope,
            vec![doc! {"$set":{"phase_observations":{"$slice":[{"$concatArrays":[
                {"$filter":{"input":{"$ifNull":["$phase_observations",[]]},"as":"p","cond":{"$not":[{"$in":["$$p.turn_id",&ids]}]}}},
                {"$literal":values}]},-(MAX_OBSERVATIONS as i64)]}}}]).await
    }).await;
}

pub fn offset(call: &VoiceSession) -> i64 {
    (Utc::now() - call.created_at).num_milliseconds()
}

pub(super) fn announcement_matches(
    segment: &super::transcript::Segment,
    text: &str,
    announced_ms: i64,
) -> bool {
    segment.speaker == super::transcript::Speaker::Assistant
        && segment.sealed
        && segment.complete
        && !segment.interrupted
        && segment.start_ms >= announced_ms
        && segment.end_ms > segment.start_ms
        && super::confirmation::same_speech(&segment.text, text)
}

#[derive(Default)]
pub struct PendingPlayback(std::collections::VecDeque<(String, ObservationKind, i64, i64, Report)>);
impl PendingPlayback {
    pub fn push(
        &mut self,
        id: String,
        kind: ObservationKind,
        start: i64,
        end: i64,
        report: Report,
    ) {
        if self.0.len() >= MAX_OBSERVATIONS {
            self.0.pop_front();
        }
        if start >= 0 && end > start {
            self.0.push_back((id, kind, start, end, report));
        }
    }
    pub async fn confirm(
        &mut self,
        db: &Database,
        call: &VoiceSession,
        watermark: i64,
        inaudible_until: i64,
    ) {
        let mut remaining = std::collections::VecDeque::new();
        let mut confirmed = Vec::new();
        while let Some((id, kind, start, end, mut report)) = self.0.pop_front() {
            if watermark >= end && inaudible_until <= start {
                report.add(
                    Phase::PlaybackConfirmed,
                    Source::ClientPlayback,
                    Kind::Offset,
                    offset(call),
                );
                confirmed.push(Observation {
                    turn_id: id,
                    related_turn_id: None,
                    kind,
                    phase_timing: report.finish(),
                });
            } else if inaudible_until <= start {
                remaining.push_back((id, kind, start, end, report));
            }
        }
        self.0 = remaining;
        record_batch(db, call, confirmed).await;
    }
}

/// Scoped task reads reuse already persisted duration snapshots, without new scans.
pub async fn for_status(
    db: &Database,
    request: &VoiceRequest,
    timing: Option<&TurnTiming>,
    activities: &[TurnActivity],
) -> AppResult<Report> {
    if timing.is_none() && activities.is_empty() {
        return Ok(request_report(request));
    }
    let call = db.collection::<bson::Document>(COLLECTION_NAME).find_one(doc!{
        "_id":&request.session_id,"user_id":&request.user_id,"conversation_id":&request.conversation_id
    }).projection(doc!{"created_at":1}).max_time(std::time::Duration::from_secs(2)).await?;
    let mut request = request.clone();
    if let Some(start) = call
        .as_ref()
        .and_then(|c| c.get_datetime("created_at").ok())
    {
        request.execution_timing = Some(execution_snapshot(
            &request.turn_id,
            start.to_chrono(),
            timing,
            activities,
        ));
    }
    Ok(request_report(&request))
}
