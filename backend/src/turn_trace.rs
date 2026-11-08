//! Turn correlation is diagnostic metadata, never execution authority.
//! The bounded writer accepts this closed envelope only, not tracing fields,
//! headers, error strings, or request/response values from callers.
use std::{
    future::Future,
    io::Write,
    pin::Pin,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    task::{Context, Poll},
    time::Instant,
};

use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderValue},
    response::Response,
};
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use serde::{Serialize, Serializer};
use uuid::Uuid;

use crate::errors::AppResult;

pub(crate) const TARGET: &str = "nyxid::turn_trace";
pub(crate) const REQUEST_HEADER: &str = "x-request-id";
const CAPACITY: usize = 4096;
const MAX_RECORD_BYTES: usize = 2048;
static WRITER: OnceLock<mpsc::SyncSender<Record>> = OnceLock::new();
static INSTANCE: OnceLock<Uuid> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);

tokio::task_local! {
    static CURRENT: Option<TurnContext>;
    static TURN_CALLBACK: bool;
    static INGRESS: Ingress;
    #[cfg(test)]
    static CAPTURE: std::sync::Mutex<Vec<Record>>;
    #[cfg(test)]
    pub(crate) static DISABLED: bool;
    #[cfg(test)]
    static TEST_SINK: mpsc::SyncSender<Record>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TurnContext {
    turn: Uuid,
    span: u64,
    parent: Option<u64>,
    request: Uuid,
}

#[derive(Clone)]
pub(crate) struct Ingress {
    pub(crate) request_id: Uuid,
    admitted: Arc<OnceLock<TurnContext>>,
    span: u64,
    started: Instant,
}

impl Ingress {
    pub(crate) fn new() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            admitted: Arc::new(OnceLock::new()),
            span: fresh_span(),
            started: Instant::now(),
        }
    }

    pub(crate) async fn scope<T>(&self, future: impl Future<Output = T>) -> T {
        emit(self.record(Phase::Start));
        INGRESS
            .scope(
                self.clone(),
                TURN_CALLBACK.scope(false, CURRENT.scope(None, future)),
            )
            .await
    }

    fn record(&self, phase: Phase) -> Record {
        let mut record = Record::new(
            TurnContext {
                turn: Uuid::nil(),
                span: self.span,
                parent: None,
                request: self.request_id,
            },
            Stage::Ingress,
            phase,
        );
        record.turn_id = None;
        record.trace_id = None;
        record
    }

    /// Ingress timing ends at headers, never by consuming an SSE stream.
    pub(crate) fn finish(&self, status: u16) {
        let mut record = self.record(Phase::End);
        record.duration_ms = Some(self.started.elapsed().as_millis().min(u64::MAX as u128) as u64);
        record.http_status = Some(status);
        record.outcome = Some(if status < 400 {
            Outcome::Completed
        } else {
            Outcome::Failed
        });
        emit(record);
    }

    pub(crate) fn admitted(&self) -> Option<TurnContext> {
        self.admitted.get().copied()
    }
}

fn fresh_span() -> u64 {
    u64::from_be_bytes(
        Uuid::new_v4().as_bytes()[..8]
            .try_into()
            .expect("eight bytes"),
    )
    .max(1)
}

pub(crate) fn ingress_request_id() -> Option<String> {
    INGRESS
        .try_with(|ingress| ingress.request_id.to_string())
        .ok()
}

impl TurnContext {
    /// Only call with a server-admitted turn, not a request/header/tool argument.
    pub(crate) fn for_turn(turn: &str) -> Option<Self> {
        #[cfg(test)]
        if DISABLED.try_with(|disabled| *disabled).unwrap_or(false) {
            return None;
        }
        let turn = Uuid::parse_str(turn)
            .ok()
            .filter(|id| id.get_version_num() == 4 && id.get_variant() == uuid::Variant::RFC4122)?;
        // Stable root across worker recovery. Domain separation is diagnostic,
        // not an authentication MAC; a turn ID conveys no authority.
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest([b"nyxid-turn-root-v1:".as_slice(), turn.as_bytes()].concat());
        let span = u64::from_be_bytes(hash[..8].try_into().expect("eight bytes")).max(1);
        Some(Self {
            turn,
            span,
            parent: None,
            request: Uuid::new_v4(),
        })
    }

    pub(crate) fn current() -> Option<Self> {
        CURRENT.try_with(|value| *value).ok().flatten()
    }

    pub(crate) fn turn_id(self) -> String {
        self.turn.to_string()
    }

    pub(crate) fn request_id(self) -> Uuid {
        self.request
    }

    pub(crate) fn child(self) -> Self {
        Self {
            span: fresh_span(),
            parent: Some(self.span),
            request: Uuid::new_v4(),
            ..self
        }
    }

    /// Authentication must already have loaded the exact turn. A reusable
    /// conversation key without matching propagation is deliberately unlinked.
    pub(crate) fn callback(headers: &HeaderMap, admitted_turn: Option<&str>) -> Option<Self> {
        let root = Self::for_turn(admitted_turn?)?;
        if headers
            .get("traceparent")
            .is_some_and(|value| value.as_bytes().len() > 256)
            || !crate::services::proxy_service::valid_traceparent_header(headers)
        {
            uncorrelated(if headers.contains_key("traceparent") {
                GapReason::InvalidParent
            } else {
                GapReason::MissingParent
            });
            return None;
        }
        let value = headers.get("traceparent")?.to_str().ok()?;
        if value[3..35] != root.turn.simple().to_string() {
            uncorrelated(GapReason::MismatchedTurn);
            return None;
        }
        let parent = u64::from_str_radix(&value[36..52], 16).ok()?;
        Some(Self {
            span: parent,
            ..root
        })
    }

    pub(crate) fn for_request(headers: &HeaderMap, admitted_turn: Option<&str>) -> RequestContext {
        RequestContext {
            turn: admitted_turn.and_then(|turn| {
                Self::current()
                    .filter(|context| context.turn_id() == turn)
                    .or_else(|| Self::callback(headers, Some(turn)))
            }),
            turn_callback: admitted_turn.is_some() || turn_callback(),
        }
    }

    pub(crate) fn traceparent(self) -> String {
        format!("00-{}-{:016x}-01", self.turn.simple(), self.span)
    }

    /// Apply at actual egress, after semantic identity/approval validation.
    pub(crate) fn inject(self, headers: &mut Vec<(String, String)>) {
        clear_correlation(headers);
        headers.push(("traceparent".into(), self.traceparent()));
        headers.push((REQUEST_HEADER.into(), self.request.to_string()));
    }

    pub(crate) fn response_header(self, response: &mut Response) {
        response.extensions_mut().insert(self);
        response.headers_mut().insert(
            crate::mw::assistant_trace::TRACE_HEADER,
            HeaderValue::from_str(&self.turn.to_string()).expect("UUID header"),
        );
    }
}

/// An authenticated turn callback may be uncorrelated (old or stale peer).
/// Retain that distinction from ordinary proxy traffic until actual egress.
#[derive(Clone, Copy)]
pub(crate) struct RequestContext {
    turn: Option<TurnContext>,
    turn_callback: bool,
}

impl RequestContext {
    pub(crate) fn context(self) -> Option<TurnContext> {
        self.turn
    }

    pub(crate) async fn scope<T>(self, future: impl Future<Output = T>) -> T {
        TURN_CALLBACK
            .scope(self.turn_callback, scope(self.turn, future))
            .await
    }

    pub(crate) async fn response(
        self,
        stage: Stage,
        future: impl Future<Output = AppResult<Response>>,
    ) -> AppResult<Response> {
        TURN_CALLBACK
            .scope(self.turn_callback, response(self.turn, stage, future))
            .await
    }
}

fn turn_callback() -> bool {
    TURN_CALLBACK.try_with(|value| *value).unwrap_or(false)
}

fn is_correlation_header(name: &str) -> bool {
    ["traceparent", "tracestate", "baggage", REQUEST_HEADER]
        .iter()
        .any(|header| name.eq_ignore_ascii_case(header))
}

fn clear_correlation(headers: &mut Vec<(String, String)>) {
    headers.retain(|(name, _)| !is_correlation_header(name));
}

/// Preserve ordinary proxy forwarding, but never forward an untrusted turn
/// claim from a late/missing-context callback. Apply after semantic identity.
pub(crate) fn inject_egress(headers: &mut Vec<(String, String)>) {
    if let Some(context) = TurnContext::current() {
        context.inject(headers);
    } else if turn_callback() {
        clear_correlation(headers);
        headers.push((REQUEST_HEADER.into(), Uuid::new_v4().to_string()));
    }
}

/// Keep credential-bearing values inside their zeroizing transport wrapper.
pub(crate) fn inject_secret_egress(
    headers: &mut crate::models::default_request_header::SecretHeaders,
) {
    let mut correlation = Vec::new();
    inject_egress(&mut correlation);
    if !correlation.is_empty() {
        headers.retain(|(name, _)| !is_correlation_header(name));
        headers.extend(correlation);
    }
}

/// Capture explicitly before handing an admitted task to Tokio. Task locals
/// otherwise disappear at spawn; this carries no credentials or content.
pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let ingress = INGRESS.try_with(Clone::clone).ok();
    let callback = turn_callback();
    let context = TurnContext::current().or_else(|| {
        (!callback)
            .then(|| ingress.as_ref().and_then(Ingress::admitted))
            .flatten()
    });
    tokio::spawn(TURN_CALLBACK.scope(callback, async move {
        match ingress {
            Some(ingress) => INGRESS.scope(ingress, scope(context, future)).await,
            None => scope(context, future).await,
        }
    }))
}

pub(crate) async fn scope<T>(context: Option<TurnContext>, future: impl Future<Output = T>) -> T {
    CURRENT.scope(context, future).await
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Ingress,
    Admission,
    Execution,
    CapacityWait,
    Engine,
    FirstOutput,
    Mcp,
    Proxy,
    ProxyHeaders,
    ProxyBody,
    NodeHeaders,
    Channel,
    ChannelPart,
    Replay,
    Gateway,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Completed,
    Failed,
    Cancelled,
    Unknown,
    Streamed,
    Silent,
    Confirmed,
    Rejected,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Start,
    End,
    Link,
    Milestone,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum GapReason {
    MissingParent,
    InvalidParent,
    MismatchedTurn,
}

fn uncorrelated(reason: GapReason) {
    let ingress = Ingress::new();
    let mut record = ingress.record(Phase::Milestone);
    record.gap_reason = Some(reason);
    emit(record);
}

fn trace_id<S: Serializer>(id: &Option<Uuid>, serializer: S) -> Result<S::Ok, S::Error> {
    match id {
        Some(id) => serializer.collect_str(&id.simple()),
        None => serializer.serialize_none(),
    }
}
fn span_id<S: Serializer>(id: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&format_args!("{id:016x}"))
}

fn optional_span_id<S: Serializer>(id: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
    match id {
        Some(id) => span_id(id, serializer),
        None => serializer.serialize_none(),
    }
}

/// Fixed-size metadata only: formatting and output allocation stay on the writer.
#[derive(Clone, Copy, Debug, Serialize)]
struct Record {
    schema_version: u8,
    target: &'static str,
    service: &'static str,
    instance_id: Uuid,
    timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<Uuid>,
    #[serde(serialize_with = "trace_id", skip_serializing_if = "Option::is_none")]
    trace_id: Option<Uuid>,
    #[serde(serialize_with = "span_id")]
    span_id: u64,
    #[serde(
        serialize_with = "optional_span_id",
        skip_serializing_if = "Option::is_none"
    )]
    parent_span_id: Option<u64>,
    request_id: Uuid,
    phase: Phase,
    stage: Stage,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ingress_request_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    related_turn_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gap_reason: Option<GapReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    part_index: Option<u32>,
}

impl Record {
    fn new(context: TurnContext, stage: Stage, phase: Phase) -> Self {
        Self {
            schema_version: 1,
            target: TARGET,
            service: "nyxid",
            instance_id: *INSTANCE.get_or_init(Uuid::new_v4),
            timestamp: chrono::Utc::now(),
            turn_id: Some(context.turn),
            trace_id: Some(context.turn),
            span_id: context.span,
            parent_span_id: context.parent,
            request_id: context.request,
            phase,
            stage,
            duration_ms: None,
            outcome: None,
            http_status: None,
            error_code: None,
            ingress_request_id: None,
            related_turn_id: None,
            gap_reason: None,
            part_index: None,
        }
    }
}

/// Only the closed record schema can reach the reusable NDJSON output buffer.
fn encode(record: &Record, bytes: &mut Vec<u8>) -> bool {
    bytes.clear();
    if serde_json::to_writer(&mut *bytes, record).is_err() || bytes.len() >= MAX_RECORD_BYTES {
        return false;
    }
    bytes.push(b'\n');
    true
}

/// Install once at process startup. No request waits for stdout or a collector.
pub(crate) fn init() {
    let (sender, receiver) = mpsc::sync_channel(CAPACITY);
    if WRITER.set(sender).is_err() {
        return;
    }
    let worker = std::thread::Builder::new()
        .name("turn-trace-writer".into())
        .spawn(move || {
            let output = std::io::stdout();
            let mut bytes = Vec::with_capacity(MAX_RECORD_BYTES);
            let mut last_warning = Instant::now();
            let mut previous_dropped = 0;
            while let Ok(record) = receiver.recv() {
                if !encode(&record, &mut bytes) || output.lock().write_all(&bytes).is_err() {
                    DROPPED.fetch_add(1, Ordering::Relaxed);
                }
                let dropped = DROPPED.load(Ordering::Relaxed);
                if dropped > previous_dropped && last_warning.elapsed().as_secs() >= 60 {
                    // This runs on the writer thread, never on an execution task.
                    tracing::warn!(target: "nyxid::turn_trace_health", dropped_records = dropped,
                    "Turn trace coverage degraded");
                    previous_dropped = dropped;
                    last_warning = Instant::now();
                }
            }
        });
    if worker.is_err() {
        tracing::error!(target: "nyxid::turn_trace_health", "Turn trace writer unavailable");
    }
}

fn emit(record: Record) {
    #[cfg(test)]
    if DISABLED.try_with(|disabled| *disabled).unwrap_or(false) {
        return;
    }
    #[cfg(test)]
    if CAPTURE
        .try_with(|capture| capture.lock().unwrap().push(record))
        .is_ok()
    {
        return;
    }
    #[cfg(test)]
    if TEST_SINK
        .try_with(|writer| {
            if writer.try_send(record).is_err() {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        })
        .is_ok()
    {
        return;
    }
    if let Some(writer) = WRITER.get()
        && writer.try_send(record).is_err()
    {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

/// Call only after successful commit. Reuses existing receipt/turn identifiers.
pub(crate) fn admitted(turn: &str, started: chrono::DateTime<chrono::Utc>, retry_of: Option<&str>) {
    let Some(context) = TurnContext::for_turn(turn) else {
        return;
    };
    let mut record = Record::new(context, Stage::Admission, Phase::Start);
    record.timestamp = started;
    record.related_turn_id = retry_of
        .and_then(|id| Uuid::parse_str(id).ok())
        .or_else(|| {
            TurnContext::current()
                .filter(|parent| parent.turn != context.turn)
                .map(|parent| parent.turn)
        });
    emit(record);
    associate(context);
}

pub(crate) fn associate(context: TurnContext) {
    let _ = INGRESS.try_with(|ingress| {
        if ingress.admitted.set(context).is_err() {
            return;
        }
        let mut record = Record::new(context, Stage::Admission, Phase::Link);
        record.ingress_request_id = Some(ingress.request_id);
        emit(record);
    });
}

pub(crate) fn source_link(turn: &str, request: &str) {
    let Some(context) = TurnContext::for_turn(turn) else {
        return;
    };
    let Ok(request) = Uuid::parse_str(request) else {
        return;
    };
    if request.get_version_num() != 4 {
        return;
    }
    let mut record = Record::new(context, Stage::Admission, Phase::Link);
    record.ingress_request_id = Some(request);
    emit(record);
}

pub(crate) fn milestone(
    turn: &str,
    stage: Stage,
    duration_ms: Option<u64>,
    outcome: Option<Outcome>,
) {
    let Some(context) = TurnContext::for_turn(turn) else {
        return;
    };
    let mut record = Record::new(context, stage, Phase::Milestone);
    record.duration_ms = duration_ms;
    record.outcome = outcome;
    emit(record);
}

/// Enrich before an existing audit is hashed or its async writer is spawned.
pub(crate) fn audit_data(data: Option<serde_json::Value>) -> Option<serde_json::Value> {
    let Some(context) = TurnContext::current().or_else(|| {
        (!turn_callback())
            .then(|| INGRESS.try_with(Ingress::admitted).ok().flatten())
            .flatten()
    }) else {
        return data;
    };
    let mut data = data.unwrap_or_else(|| serde_json::json!({}));
    if let Some(object) = data.as_object_mut() {
        // Explicit persisted source-turn metadata wins over ambient work (e.g.
        // retiring a reclaimed turn during admission of its replacement).
        if object
            .get("turn_id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id != context.turn_id())
        {
            return Some(data);
        }
        object.insert("turn_id".into(), context.turn_id().into());
        object.insert("trace_id".into(), context.turn.simple().to_string().into());
        object.insert(
            "trace_span_id".into(),
            format!("{:016x}", context.span).into(),
        );
        object.insert(
            "trace_request_id".into(),
            context.request.to_string().into(),
        );
    }
    Some(data)
}

pub(crate) struct Operation {
    context: Option<TurnContext>,
    stage: Stage,
    started: Instant,
    finished: bool,
    part_index: Option<u32>,
}

impl Operation {
    pub(crate) fn start(context: Option<TurnContext>, stage: Stage) -> Self {
        Self::new(context, stage, None)
    }
    pub(crate) fn part(context: Option<TurnContext>, index: usize) -> Self {
        Self::new(context, Stage::ChannelPart, u32::try_from(index).ok())
    }
    fn new(context: Option<TurnContext>, stage: Stage, part_index: Option<u32>) -> Self {
        let context = context.map(TurnContext::child);
        if let Some(context) = context {
            let mut record = Record::new(context, stage, Phase::Start);
            record.part_index = part_index;
            emit(record);
        }
        Self {
            context,
            stage,
            started: Instant::now(),
            finished: false,
            part_index,
        }
    }
    pub(crate) fn context(&self) -> Option<TurnContext> {
        self.context
    }
    pub(crate) fn finish(&mut self, outcome: Outcome, status: Option<u16>, error: Option<u32>) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(context) = self.context {
            let mut record = Record::new(context, self.stage, Phase::End);
            record.part_index = self.part_index;
            record.duration_ms =
                Some(self.started.elapsed().as_millis().min(u64::MAX as u128) as u64);
            record.outcome = Some(outcome);
            record.http_status = status;
            record.error_code = error;
            emit(record);
        }
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        self.finish(Outcome::Unknown, None, None);
    }
}

pub(crate) async fn response(
    context: Option<TurnContext>,
    stage: Stage,
    future: impl Future<Output = AppResult<Response>>,
) -> AppResult<Response> {
    let mut operation = Operation::start(context, stage);
    let result = scope(operation.context(), future).await;
    match result {
        Ok(response) if operation.context.is_some() || turn_callback() => {
            let status = response.status().as_u16();
            let (parts, body) = response.into_parts();
            if body.is_end_stream() {
                operation.finish(
                    response_outcome(status, operation.stage),
                    Some(status),
                    None,
                );
            }
            Ok(Response::from_parts(
                parts,
                Body::new(TraceBody {
                    body: Box::pin(body),
                    operation,
                    status,
                    turn_callback: turn_callback(),
                }),
            ))
        }
        Ok(response) => Ok(response),
        Err(error) => {
            operation.finish(
                Outcome::Failed,
                Some(error.status_code().as_u16()),
                Some(error.error_code()),
            );
            Err(error)
        }
    }
}

fn response_outcome(status: u16, stage: Stage) -> Outcome {
    if status >= 400 {
        Outcome::Failed
    } else if matches!(stage, Stage::Replay) {
        Outcome::Streamed
    } else {
        Outcome::Completed
    }
}

struct TraceBody {
    body: Pin<Box<Body>>,
    operation: Operation,
    status: u16,
    turn_callback: bool,
}
impl HttpBody for TraceBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let context = self.operation.context();
        let callback = self.turn_callback;
        let result = TURN_CALLBACK.sync_scope(callback, || {
            CURRENT.sync_scope(context, || self.body.as_mut().poll_frame(cx))
        });
        match &result {
            Poll::Ready(None) => {
                let status = self.status;
                let outcome = response_outcome(status, self.operation.stage);
                self.operation.finish(outcome, Some(status), None);
            }
            Poll::Ready(Some(Err(_))) => self.operation.finish(Outcome::Failed, None, None),
            Poll::Ready(Some(Ok(_))) if self.body.is_end_stream() => {
                let status = self.status;
                let outcome = response_outcome(status, self.operation.stage);
                self.operation.finish(outcome, Some(status), None);
            }
            _ => {}
        }
        result
    }
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

#[cfg(test)]
#[path = "turn_trace_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) async fn capture<T>(future: impl Future<Output = T>) -> (T, Vec<serde_json::Value>) {
    CAPTURE
        .scope(std::sync::Mutex::new(Vec::new()), async {
            let result = future.await;
            let rows = CAPTURE.with(|rows| {
                rows.lock()
                    .unwrap()
                    .iter()
                    .map(|record| serde_json::to_value(record).unwrap())
                    .collect()
            });
            (result, rows)
        })
        .await
}
