//! Owner-only negotiated Responses control. This is separate from automatic
//! Read and from repeatable Admission::revalidate. No expiry rearms a dispatch.
use bson::{Document, doc};
use chrono::{Duration, Utc};
use mongodb::{ClientSession, Database};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{assistant_acknowledgement_service as acks, assistant_model_control_protocol as wire};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_model_control::{COLLECTION_NAME, ModelControlRoot},
};

mod callback;
mod compact;
mod envelope;
mod issuance;
mod tools;
pub(crate) use tools::{tool_claim, tool_complete, tool_prepare, tool_status};
mod observer;
mod persistence;
mod read_observation;
pub(crate) use read_observation::admit_recorder_in_session;
pub(crate) use read_observation::unobserved_http;
mod redirect_observation;
pub use redirect_observation::HttpClient;
mod initial_compact;
#[cfg(test)]
mod tests;
pub(crate) use callback::{
    Callback, CallbackRequest, callback, decision_in_session, observe, require_control_lineage,
    require_http_control_lineage, require_unactivated_exact_only,
};
pub(crate) use issuance::{IssueRequest, issue};
pub(crate) use observer::CompletionObserver;
pub use persistence::ensure_indexes;

pub const READINESS_CAPABILITY: &str = "assistant:model-control-initial-compact-recorder-v1";
pub const AUDIENCE: &str = "nyxagent-model-control";
const TOKEN_TYPE: &str = "nyxid_owner_model_control";
const MAX_STEPS: usize = 128;
// The entire encrypted root is a MongoDB document (16 MiB hard limit). Leave
// at least half that limit for encryption envelopes and public row overhead.
// Exhaustion refuses preparation; it never creates a replacement logical root.
const MAX_PRIVATE_BYTES: usize = 8 * 1024 * 1024;

/// Trusted in-process target binding accompanies the secret through both owner
/// egress adapters. A slug re-resolution cannot move a control to a new row.
pub struct TargetedControl {
    token: wire::SecretToken,
    runtime_catalog_id: String,
}
impl TargetedControl {
    pub(crate) fn for_target(&self, catalog_id: &str) -> AppResult<&wire::SecretToken> {
        if self.runtime_catalog_id != catalog_id {
            return Err(invalid());
        }
        Ok(&self.token)
    }
}

fn digest(value: Value) -> String {
    super::mcp_service::canonical_sha256(value)
}
fn token_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn conflict() -> AppError {
    AppError::Conflict(
        "Model control state conflicts; this logical action will not be dispatched again".into(),
    )
}
fn unsupported() -> AppError {
    AppError::Conflict("This model request has no validated transition in its approved Responses and standalone-compaction family".into())
}
fn invalid() -> AppError {
    AppError::Forbidden("Invalid or expired model control capability".into())
}

#[derive(Clone, Serialize, Deserialize)]
struct BoundControl {
    claims: wire::ControlClaims,
    token: wire::SecretToken,
    /// Exact server-authored outer input. It is not supplied by prepare.
    resumed_input: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct RootPrivate {
    controls: Vec<BoundControl>,
    envelope: Option<envelope::Envelope>,
    initial_input: Vec<Value>,
    connection_id: Option<String>,
    connection_binding: Option<String>,
    owner_id: Option<String>,
    catalog_id: Option<String>,
    card_arguments: Option<Value>,
    effective_headers_binding: Option<String>,
    steps: Vec<Step>,
    #[serde(default)]
    preconsent: read_observation::State,
    #[serde(default)]
    initial_operation: StepOperation,
    #[serde(default)]
    initial_request_binding: Option<String>,
}
impl RootPrivate {
    fn tool_steps(&self) -> impl Iterator<Item = &Step> {
        self.steps
            .iter()
            .chain(self.preconsent.reads.iter().map(|read| &read.record))
    }
    fn active_tool_step_mut(&mut self, phase: &str, control: &str) -> AppResult<&mut Step> {
        if phase == "allowed" {
            return self.steps.last_mut().ok_or_else(conflict);
        }
        if phase == "unactivated" && !self.preconsent.ineligible {
            let read = self
                .preconsent
                .reads
                .last_mut()
                .filter(|read| read.control_hash == control)
                .ok_or_else(conflict)?;
            return Ok(&mut read.record);
        }
        Err(conflict())
    }
    fn tool_step_mut(&mut self, id: &str) -> AppResult<&mut Step> {
        if let Some(step) = self.steps.iter_mut().find(|step| step.id == id) {
            return Ok(step);
        }
        self.preconsent
            .reads
            .iter_mut()
            .find(|read| read.record.id == id)
            .map(|read| &mut read.record)
            .ok_or_else(conflict)
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Step {
    id: String,
    ordinal: u32,
    request_binding: String,
    input: Vec<Value>,
    capability_hash: String,
    capability_ciphertext: Vec<u8>,
    phase: StepPhase,
    #[serde(default)]
    operation: StepOperation,
    #[serde(default)]
    tools: Vec<HostTool>,
}
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StepOperation {
    #[default]
    Generation,
    Compact,
}
#[derive(Clone, Serialize, Deserialize)]
struct HostTool {
    call_id: String,
    capability: wire::SecretToken,
    #[serde(default = "wire::default_tool_result_chars")]
    result_char_limit: usize,
    #[serde(default)]
    control_hash: Option<String>,
    #[serde(default)]
    refusal_evidence: Option<RefusalEvidence>,
    #[serde(default)]
    refusal_source_hash: Option<String>,
    phase: HostToolPhase,
}
#[derive(Clone, Serialize, Deserialize)]
struct RefusalEvidence {
    kind: wire::RefusalKind,
    target_name: String,
    target_arguments_hash: String,
    attempt_hash: String,
    origin_step_id: String,
    origin_call_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum HostToolPhase {
    Prepared,
    Dispatched { dispatch_id: String },
    Completed { receipt: wire::ToolResultReceipt },
    Unsupported,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum StepPhase {
    Prepared,
    /// Already irreversible even if the process dies before marking unknown.
    Dispatched {
        dispatch_id: String,
    },
    Completed {
        completion: ObservedCompletion,
        receipt: wire::CompletionReceipt,
    },
    OutcomeUnknown {
        dispatch_id: String,
    },
}
#[derive(Clone, Serialize, Deserialize)]
struct ObservedCompletion {
    response_id: String,
    output: Vec<Value>,
    calls: Vec<ProviderToolCall>,
    #[serde(default)]
    compaction: Option<ObservedCompaction>,
}
#[derive(Clone, Serialize, Deserialize)]
struct ObservedCompaction {
    response: Value,
    body_sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct ProviderToolCall {
    call_id: String,
    name: String,
    arguments: String,
}
/// Constructed only by the actual server response observer.
struct VerifiedProviderCompletion {
    response_id: String,
    output: Vec<Value>,
    calls: Vec<ProviderToolCall>,
}
/// Only server MCP evidence can construct these. Not a wire input.
struct VerifiedToolResult {
    call_id: String,
    tool_name: String,
    arguments_binding: String,
    model_text_binding: String,
}

/// Held server admission, not caller-deserializable and not a consumable
/// revalidation. The final send transaction checks its durable step once.
#[derive(Clone)]
pub(crate) struct PreparedDispatch {
    state: AppState,
    root_id: String,
    step_id: String,
    request_binding: String,
    control_hash: String,
    step_hash: String,
    chat: acks::ChatAuthority,
    connection_id: String,
    connection_binding: String,
    operation: StepOperation,
    read_observation: bool,
}
impl PreparedDispatch {
    pub(crate) fn matches_read_admission(&self, connection: &str, binding: &str) -> bool {
        self.read_observation
            && self.connection_id == connection
            && self.connection_binding == binding
    }
    pub(crate) fn requires_no_redirect(&self) -> bool {
        !self.read_observation
    }

    pub(crate) async fn disable_observation(&self) -> AppResult<()> {
        if !self.read_observation {
            return Err(conflict());
        }
        read_observation::disable(&self.state, &self.chat, &self.root_id).await
    }
    pub(crate) async fn claim(&self) -> AppResult<CompletionObserver> {
        let claim = if self.read_observation {
            read_observation::claim(&self.state, self).await?
        } else {
            final_send_claim(&self.state, self).await?
        };
        Ok(CompletionObserver::new(self.state.clone(), claim))
    }
}
pub(crate) struct ClaimedDispatch {
    root_id: String,
    step_id: String,
    dispatch_id: String,
    operation: StepOperation,
    read_observation: bool,
}

pub(crate) async fn verify(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    request: wire::VerifyRequest,
) -> AppResult<wire::VerifyReply> {
    if request.protocol != wire::PROTOCOL {
        return Err(invalid());
    }
    let (row, private, control) = persistence::authenticated(state, chat, token, None).await?;
    if control.claims.outer_request_binding != request.outer_request_binding {
        return Err(invalid());
    }
    // Decrypted data is never serialized into a public Responses record.
    let _ = private;
    Ok(wire::VerifyReply {
        claims: control.claims,
        activated: row.phase != "unactivated",
    })
}

pub(crate) async fn status(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    request: wire::StatusRequest,
) -> AppResult<wire::PrepareReply> {
    if request.protocol != wire::PROTOCOL
        || (request.step_id.is_some() && request.request_binding.is_none())
    {
        return Err(invalid());
    }
    let (row, private, _) = persistence::authenticated(state, chat, token, None).await?;
    if row.id != request.root_id {
        return Err(invalid());
    }
    if row.phase == "unactivated" && request.step_id.is_none() && request.request_binding.is_some()
    {
        return Ok(read_observation::status(
            &private,
            request.request_binding.as_deref(),
            &token_hash(token.expose()),
        ));
    }
    if row.phase == "allowed"
        && private.steps.is_empty()
        && private.initial_operation == StepOperation::Compact
    {
        return Ok(wire::PrepareReply::InitialCompact);
    }
    if let Some(id) = request.step_id.as_deref() {
        let step = private
            .steps
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(invalid)?;
        if Some(step.request_binding.as_str()) != request.request_binding.as_deref() {
            return Err(conflict());
        }
        return persistence::step_reply(state, &row, step).await;
    }
    // Lost prepare reply: the same control/root may retrieve its current step,
    // never create a fresh token or infer a new step from a changed body.
    if let Some(step) = private.steps.last() {
        return persistence::step_reply(state, &row, step).await;
    }
    root_reply(&row)
}

fn root_reply(row: &ModelControlRoot) -> AppResult<wire::PrepareReply> {
    match row.phase.as_str() {
        "unactivated" => Ok(wire::PrepareReply::Observe),
        "awaiting_decision" => Ok(wire::PrepareReply::AwaitingDecision {
            acknowledgement_id: row.acknowledgement_id.clone().ok_or_else(conflict)?,
        }),
        "denied" | "revoked" => Ok(wire::PrepareReply::Denied),
        _ => Err(conflict()),
    }
}

pub(crate) async fn prepare(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    candidate: wire::PrepareRequest,
) -> AppResult<wire::PrepareReply> {
    let (row, mut private, control) = persistence::authenticated(state, chat, token, None).await?;
    if row.phase == "unactivated" && candidate.relative_path == "responses/compact" {
        return initial_compact::prepare(state, chat, token, &row, &private, &candidate).await;
    }
    if row.phase == "unactivated"
        && super::assistant_service_execution::stateless_inference(
            &candidate.relative_path,
            &candidate.body,
        )
    {
        return Ok(wire::PrepareReply::AutomaticRead);
    }
    let parsed = match private
        .envelope
        .as_ref()
        .filter(|_| row.phase != "unactivated")
    {
        Some(saved) => envelope::within_family(saved, &candidate)
            .map(|(operation, input)| (saved.clone(), input, operation)),
        None => envelope::envelope(&candidate)
            .map(|(envelope, input)| (envelope, input, StepOperation::Generation)),
    };
    let (envelope, input, operation) = match parsed {
        Ok(candidate) => candidate,
        Err(_) if row.phase == "unactivated" => return Ok(wire::PrepareReply::ExactOnly {
            reason:"Reusable Responses v1 does not cover this metadata, compaction or executable shape. Existing exact approval applies.".into() }),
        Err(error) => return Err(error),
    };
    if row.phase == "unactivated" {
        return Ok(wire::PrepareReply::Observe);
    }
    if matches!(row.phase.as_str(), "denied" | "revoked") {
        return Ok(wire::PrepareReply::Denied);
    }
    if private.envelope.as_ref() != Some(&envelope) {
        return Err(unsupported());
    }
    if row.phase == "awaiting_decision" {
        return root_reply(&row);
    }
    if row.phase != "allowed" || private.steps.len() >= MAX_STEPS {
        return Err(conflict());
    }
    let binding = envelope::request_binding(&candidate);
    if let Some(step) = private.steps.last() {
        match &step.phase {
            StepPhase::Prepared if step.request_binding == binding => {
                return persistence::step_reply(state, &row, step).await;
            }
            StepPhase::Prepared => return Err(conflict()),
            StepPhase::Dispatched { .. } => {
                return Ok(wire::PrepareReply::InProgress {
                    step_id: step.id.clone(),
                });
            }
            StepPhase::OutcomeUnknown { .. } => return Ok(wire::PrepareReply::OutcomeUnknown),
            StepPhase::Completed {
                completion,
                receipt,
            } => {
                if step.request_binding == binding {
                    return Ok(wire::PrepareReply::Completed {
                        receipt: receipt.clone(),
                    });
                }
                if candidate.preceding_receipt.as_ref().is_none_or(|token| {
                    token_hash(token.expose()) != token_hash(receipt.receipt_bearer.expose())
                }) {
                    return Err(conflict());
                }
                if completion.compaction.is_some() {
                    // Canonical standalone output replaces the whole context.
                    // No pruning, projections, additions or historical tool
                    // receipts can turn this replacement into new authority.
                    if input != completion.output || !candidate.tool_results.is_empty() {
                        return Err(conflict());
                    }
                } else {
                    let results = persistence::verify_tool_results(step, &candidate.tool_results)?;
                    envelope::next_input(&step.input, completion, &input, &results)?;
                }
            }
        }
    } else {
        let saved_allow = match &control.claims.origin {
            wire::ControlOrigin::AllowedContinuation {acknowledgement_id,..} => row.acknowledgement_id.as_ref() == Some(acknowledgement_id),
            wire::ControlOrigin::Retry {original_turn_id,..} => private.controls.iter().any(|prior|
                &prior.claims.active_turn_id == original_turn_id && matches!(&prior.claims.origin,
                    wire::ControlOrigin::AllowedContinuation {acknowledgement_id,..} if row.acknowledgement_id.as_ref() == Some(acknowledgement_id))),
            _ => false,
        };
        if operation != private.initial_operation
            || !saved_allow
            || candidate.preceding_receipt.is_some()
            || !candidate.tool_results.is_empty()
        {
            return Err(conflict());
        }
        if operation == StepOperation::Compact {
            if private.initial_request_binding.as_deref() != Some(binding.as_str())
                || input != private.initial_input
            {
                return Err(conflict());
            }
        } else {
            envelope::allowed_resume(&private.initial_input, &input, &control.resumed_input)?;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let token = persistence::new_token()?;
    let ciphertext = state
        .encryption_keys
        .encrypt_with_aad(
            token.expose().as_bytes(),
            persistence::step_aad(&row, &id).as_slice(),
        )
        .await?;
    let step = Step {
        id,
        ordinal: private.steps.len() as u32,
        request_binding: binding,
        input,
        capability_hash: token_hash(token.expose()),
        capability_ciphertext: ciphertext,
        phase: StepPhase::Prepared,
        operation,
        tools: vec![],
    };
    private.steps.push(step.clone());
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async |session| {
            let result = async {
                let (live, _, _) = acks::fence(&state.db, chat, session).await?;
                super::assistant_access_policy::require_same_live_turn(chat, &live)?;
                let (mut current, _) =
                    persistence::load(state, &row.id, Some(&mut *session)).await?;
                persistence::require_row(&current, chat)?;
                if current.revision != row.revision || current.phase != "allowed" {
                    return Err(conflict());
                }
                // Reconstruct each retry's mutable row from durable state. A
                // transaction abort must not reuse an incremented local revision.
                persistence::save(state, &mut current, &private, session).await
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)?;
    persistence::step_reply(state, &row, &step).await
}

pub(crate) async fn final_send_claim(
    state: &AppState,
    prepared: &PreparedDispatch,
) -> AppResult<ClaimedDispatch> {
    #[cfg(test)]
    if let Ok((entered, release)) = BEFORE_FINAL_CLAIM.try_with(Clone::clone) {
        entered.send(()).await.map_err(|_| conflict())?;
        tokio::time::timeout(std::time::Duration::from_secs(15), release.acquire())
            .await
            .map_err(|_| conflict())?
            .map_err(|_| conflict())?
            .forget();
    }
    // Revalidate is repeatable. Only THIS transaction may irreversibly claim.
    let current = super::assistant_service_execution::connection_binding(
        &state.db,
        &prepared.connection_id,
        &prepared.chat.api_key_id,
    )
    .await?;
    if current != prepared.connection_binding {
        return Err(conflict());
    }
    let dispatch_id = uuid::Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async |session| {
            let result = async {
                let (live, _, _) = acks::fence(&state.db, &prepared.chat, session).await?;
                super::assistant_access_policy::require_same_live_turn(&prepared.chat, &live)?;
                let (mut row, mut private) =
                    persistence::load(state, &prepared.root_id, Some(&mut *session)).await?;
                persistence::require_row(&row, &prepared.chat)?;
                if row.phase != "allowed"
                    || !row.control_hashes.contains(&prepared.control_hash)
                    || private.connection_binding.as_deref() != Some(current.as_str())
                    || private.connection_id.as_deref() != Some(prepared.connection_id.as_str())
                {
                    return Err(conflict());
                }
                let step = private
                    .steps
                    .last_mut()
                    .filter(|s| {
                        s.id == prepared.step_id
                            && s.request_binding == prepared.request_binding
                            && s.capability_hash == prepared.step_hash
                            && s.operation == prepared.operation
                    })
                    .ok_or_else(conflict)?;
                if !matches!(step.phase, StepPhase::Prepared) {
                    return Err(conflict());
                }
                let first = step.ordinal == 0;
                step.phase = StepPhase::Dispatched {
                    dispatch_id: dispatch_id.clone(),
                };
                if first
                    && !acks::consume_action_in_session(
                        &state.db,
                        &prepared.chat,
                        row.acknowledgement_id.as_deref().ok_or_else(conflict)?,
                        "nyxid.model_control_v1",
                        private.card_arguments.as_ref().ok_or_else(conflict)?,
                        session,
                    )
                    .await?
                {
                    return Err(conflict());
                }
                persistence::save(state, &mut row, &private, session).await
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)?;
    Ok(ClaimedDispatch {
        root_id: prepared.root_id.clone(),
        step_id: prepared.step_id.clone(),
        dispatch_id,
        operation: prepared.operation,
        read_observation: false,
    })
}

#[cfg(test)]
tokio::task_local! {
    // Synchronizes real billed callback handlers immediately before the CAS;
    // it neither manufactures eligibility nor bypasses an authority check.
    static BEFORE_FINAL_CLAIM: (tokio::sync::mpsc::Sender<()>, std::sync::Arc<tokio::sync::Semaphore>);
}
