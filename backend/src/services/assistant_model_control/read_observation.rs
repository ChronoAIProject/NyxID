//! Passive facts from otherwise ordinary Read. These records never grant a
//! Write, consume a card, or provide a recoverable provider-send capability.
use super::*;
use futures::TryStreamExt;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct State {
    pub seen: bool,
    pub ineligible: bool,
    pub reads: Vec<Read>,
    pub compact: Option<InitialCompact>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Authority {
    pub connection: String,
    pub binding: String,
    pub owner: String,
    pub catalog: String,
    pub headers: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Read {
    pub envelope: envelope::Envelope,
    pub authority: Authority,
    pub control_hash: String,
    // Reuse the bounded transcript/result payload, in a separate passive
    // namespace. Capability fields are empty and are NEVER issued to a peer.
    pub record: Step,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct InitialCompact {
    pub envelope: envelope::Envelope,
    pub request_binding: String,
    pub control_hash: String,
    pub input: Vec<Value>,
    pub read_id: Option<String>,
}

impl State {
    pub fn disable(&mut self) {
        self.seen = true;
        self.ineligible = true;
        self.compact = None;
        // No Write tombstones live here. Discarding passive payload cannot
        // rearm a send or make this lineage pristine after process loss.
        self.reads.clear();
    }
}

const UNOBSERVED: &str = "model_control_unobserved";
const COVERED: &str = "model_control_recorder_admission";

#[cfg(test)]
tokio::task_local! {
    pub(super) static BEFORE_UNOBSERVED_TRANSACTION: (tokio::sync::mpsc::Sender<()>,std::sync::Arc<tokio::sync::Semaphore>);
}

fn coverage_scope(owner: &str, conversation: &str, key: &str, turn: &str) -> Document {
    doc! {"kind":COVERED,"user_id":owner,"conversation_id":conversation,
    "api_key_id":key,"turn_id":turn,"revision":READINESS_CAPABILITY}
}

/// Called only while the actual NEW initial turn is being committed. Omission
/// is safe when the serving roster is not ready; it cannot backfill old turns.
/// Once readiness succeeds, any marker write failure aborts that admission.
pub(crate) async fn admit_recorder_in_session(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
    start: &super::super::assistant_nyxagent::TurnStart,
    session: &mut ClientSession,
) -> AppResult<()> {
    if start.guest || start.retry_of.is_some() || start.acknowledgement_id.is_some() {
        return Ok(());
    }
    if !super::super::security_readiness_service::report(db, READINESS_CAPABILITY)
        .await
        .is_ok_and(|report| report.ready)
    {
        return Ok(());
    }
    let turn = row.active_turn.as_ref().ok_or_else(invalid)?;
    let mut marker = coverage_scope(
        &row.user_id,
        &row.id,
        &row.credential_api_key_id,
        &turn.turn_id,
    );
    marker.insert("_id", uuid::Uuid::new_v4().to_string());
    db.collection::<Document>(super::super::assistant_service_execution::COLLECTION)
        .insert_one(marker)
        .session(session)
        .await?;
    Ok(())
}

pub(super) async fn covered_in_session(
    state: &AppState,
    chat: &acks::ChatAuthority,
    session: &mut ClientSession,
) -> AppResult<bool> {
    Ok(state
        .db
        .collection::<Document>(super::super::assistant_service_execution::COLLECTION)
        .find_one(coverage_scope(
            &chat.user_id,
            &chat.conversation_id,
            &chat.api_key_id,
            chat.turn_id.as_deref().ok_or_else(invalid)?,
        ))
        .session(session)
        .await?
        .is_some())
}

pub(super) fn marker_scope(chat: &acks::ChatAuthority) -> AppResult<Document> {
    Ok(
        doc! {"kind":UNOBSERVED,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
        "api_key_id":&chat.api_key_id,"turn_id":chat.turn_id.as_deref().ok_or_else(invalid)?},
    )
}

fn related(private: &RootPrivate, connection: Option<&str>, catalog: &str) -> bool {
    private.catalog_id.as_deref() == Some(catalog)
        || connection.is_some_and(|id| private.connection_id.as_deref() == Some(id))
        || private.preconsent.reads.iter().any(|r| {
            r.authority.catalog == catalog || connection == Some(r.authority.connection.as_str())
        })
}

fn model_path(path: &str) -> bool {
    let path = path.trim_matches('/');
    let path = path.strip_prefix("v1/").unwrap_or(path);
    matches!(
        path,
        "responses" | "responses/compact" | "chat/completions" | "messages" | "embeddings"
    )
}

/// An omitted capability or ineligible transport is not evidence of pristine
/// history. This marker also precedes root creation. Both paths write the
/// existing live-turn fence, serializing root issuance with this observation.
/// No transcript, capability or execution grant is stored here.
pub(crate) async fn unobserved_http(
    state: &AppState,
    chat: &acks::ChatAuthority,
    connection: Option<&str>,
    target: &super::super::proxy_service::ProxyTarget,
    path: &str,
) -> AppResult<()> {
    if chat.guest || !chat.turn_live || !model_path(path) {
        return Ok(());
    }
    let root_scope = doc! {"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
    "api_key_id":&chat.api_key_id,"turn_ids":&chat.turn_id};
    if target.service.inference.is_none() {
        // A hint only, to avoid adding model fences to an unrelated effect.
        // Re-read and validate after the live fence inside every retry below.
        let rows: Vec<ModelControlRoot> = state
            .db
            .collection::<ModelControlRoot>(COLLECTION_NAME)
            .find(root_scope.clone())
            .limit(MAX_STEPS as i64 + 1)
            .await?
            .try_collect()
            .await?;
        if rows.len() > MAX_STEPS {
            return Err(conflict());
        }
        let mut found = false;
        for row in rows {
            found |= related(
                &persistence::open(state, &row).await?,
                connection,
                &target.service.id,
            );
        }
        if !found {
            return Ok(());
        }
    }
    #[cfg(test)]
    if let Ok((entered, release)) = BEFORE_UNOBSERVED_TRANSACTION.try_with(Clone::clone) {
        entered.send(()).await.map_err(|_| conflict())?;
        tokio::time::timeout(std::time::Duration::from_secs(15), release.acquire())
            .await
            .map_err(|_| conflict())?
            .map_err(|_| conflict())?
            .forget();
    }
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (state, chat, connection, target, &root_scope),
            |session, context| {
                let (state, chat, connection, target, root_scope) = *context;
                Box::pin(async move {
                    let result = async {
                        let (live, _, _) = acks::fence(&state.db, chat, session).await?;
                        super::super::assistant_access_policy::require_same_live_turn(chat, &live)?;
                        let roots = state.db.collection::<ModelControlRoot>(COLLECTION_NAME);
                        let mut cursor = roots
                            .find(root_scope.clone())
                            .limit(MAX_STEPS as i64 + 1)
                            .session(&mut *session)
                            .await?;
                        let rows: Vec<ModelControlRoot> =
                            cursor.stream(&mut *session).try_collect().await?;
                        if rows.len() > MAX_STEPS {
                            return Err(conflict());
                        }
                        let mut saved = Vec::with_capacity(rows.len());
                        for row in rows {
                            let private = persistence::open(state, &row).await?;
                            saved.push((row, private));
                        }
                        if target.service.inference.is_none()
                            && !saved.iter().any(|(_, private)| {
                                related(private, connection, &target.service.id)
                            })
                        {
                            return Ok(());
                        }
                        // A card/activation can win after the caller's earlier lineage
                        // check. Read every phase under this same fence before permitting
                        // an unobserved send; never ignore a newly saved consent family.
                        if saved.iter().any(|(row, private)| {
                            row.phase != "unactivated"
                                && (target.service.inference.is_some()
                                    || related(private, connection, &target.service.id))
                        }) {
                            return Err(conflict());
                        }
                        let scope = marker_scope(chat)?;
                        // No TTL: a long-lived turn cannot become pristine when a marker
                        // expires. Existing conversation/owner erasure removes the row.
                        state
                            .db
                            .collection::<Document>(
                                super::super::assistant_service_execution::COLLECTION,
                            )
                            .update_one(
                                scope,
                                doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string()}},
                            )
                            .upsert(true)
                            .session(&mut *session)
                            .await?;
                        for (row, private) in &mut saved {
                            if !private.preconsent.ineligible
                                && (target.service.inference.is_some()
                                    || related(private, connection, &target.service.id))
                            {
                                private.preconsent.disable();
                                persistence::save(state, row, private, session).await?;
                            }
                        }
                        Ok(())
                    }
                    .await;
                    super::super::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)
}

/// Fence inability BEFORE any ordinary Read send. A persistence error must
/// propagate; swallowing it could leave an older Completed history eligible.
pub(super) async fn disable(
    state: &AppState,
    chat: &acks::ChatAuthority,
    root: &str,
) -> AppResult<()> {
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run((state, chat, root), |session, context| {
            let (state, chat, root) = *context;
            Box::pin(async move {
                let result = async {
                    let (mut row, mut private) =
                        persistence::load(state, root, Some(&mut *session)).await?;
                    persistence::require_row(&row, chat)?;
                    if row.phase != "unactivated" {
                        return Err(conflict());
                    }
                    if !private.preconsent.ineligible {
                        private.preconsent.disable();
                        persistence::save(state, &mut row, &private, session).await?;
                    }
                    Ok(())
                }
                .await;
                super::super::api_key_mutation_service::transaction_result(result)
            })
        })
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)
}

pub(super) fn results(step: &Step) -> AppResult<Vec<VerifiedToolResult>> {
    let receipts = step
        .tools
        .iter()
        .filter_map(|tool| match &tool.phase {
            HostToolPhase::Completed { receipt } => Some(receipt.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    persistence::verify_tool_results(step, &receipts)
}

pub(super) fn continuation(read: &Read, input: &[Value]) -> AppResult<()> {
    let StepPhase::Completed { completion, .. } = &read.record.phase else {
        return Err(conflict());
    };
    envelope::next_input(
        &read.record.input,
        completion,
        input,
        &results(&read.record)?,
    )
}

/// The caller has authenticated the actual resolved callback. This only saves
/// observation intent; ordinary policy/admission still decides whether to send.
pub(super) async fn prepare(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    root: &ModelControlRoot,
    candidate: &wire::PrepareRequest,
    authority: Authority,
) -> AppResult<Option<PreparedDispatch>> {
    let parsed = envelope::envelope(candidate);
    let mut session = state.db.client().start_session().await?;
    let record = session
        .start_transaction()
        .and_run(
            (state, chat, token, root, candidate, &authority, &parsed),
            |session, context| {
                let (state, chat, token, root, candidate, authority, parsed) = *context;
                Box::pin(async move {
                    let result = async {
                        let (live, _, _) = acks::fence(&state.db, chat, session).await?;
                        super::super::assistant_access_policy::require_same_live_turn(chat, &live)?;
                        let (mut row, mut private) =
                            persistence::load(state, &root.id, Some(&mut *session)).await?;
                        persistence::require_row(&row, chat)?;
                        if row.phase != "unactivated" {
                            return Err(conflict());
                        }
                        if private.preconsent.ineligible {
                            return Ok(None);
                        }
                        let valid = parsed.as_ref().ok().filter(|(envelope, input)| {
                            private.preconsent.reads.last().is_none_or(|read| {
                                &read.envelope == envelope
                                    && &read.authority == authority
                                    && read.control_hash == token_hash(token.expose())
                                    && continuation(read, input).is_ok()
                            })
                        });
                        let Some((envelope, input)) = valid else {
                            private.preconsent.disable();
                            persistence::save(state, &mut row, &private, session).await?;
                            return Ok(None);
                        };
                        if private.preconsent.reads.len() >= MAX_STEPS {
                            private.preconsent.disable();
                            persistence::save(state, &mut row, &private, session).await?;
                            return Ok(None);
                        }
                        let record = Step {
                            id: uuid::Uuid::new_v4().to_string(),
                            ordinal: private.preconsent.reads.len() as u32,
                            request_binding: envelope::request_binding(candidate),
                            input: input.clone(),
                            capability_hash: String::new(),
                            capability_ciphertext: vec![],
                            phase: StepPhase::Prepared,
                            operation: StepOperation::Generation,
                            tools: vec![],
                        };
                        private.preconsent.seen = true;
                        private.preconsent.compact = None;
                        private.preconsent.reads.push(Read {
                            envelope: envelope.clone(),
                            authority: authority.clone(),
                            control_hash: token_hash(token.expose()),
                            record: record.clone(),
                        });
                        if serde_json::to_vec(&private).map_err(|_| invalid())?.len()
                            > MAX_PRIVATE_BYTES
                        {
                            private.preconsent.disable();
                            persistence::save(state, &mut row, &private, session).await?;
                            return Ok(None);
                        }
                        persistence::save(state, &mut row, &private, session).await?;
                        Ok(Some(record))
                    }
                    .await;
                    super::super::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)?;
    Ok(record.map(|record| PreparedDispatch {
        state: state.clone(),
        root_id: root.id.clone(),
        step_id: record.id,
        request_binding: record.request_binding,
        control_hash: token_hash(token.expose()),
        step_hash: String::new(),
        chat: chat.clone(),
        connection_id: authority.connection,
        connection_binding: authority.binding,
        operation: StepOperation::Generation,
        read_observation: true,
    }))
}

pub(super) async fn claim(
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
    let dispatch_id = uuid::Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run((state, prepared, &dispatch_id), |session, context| {
            let (state, prepared, dispatch_id) = *context;
            Box::pin(async move {
                let result = async {
                    let (live, _, _) = acks::fence(&state.db, &prepared.chat, session).await?;
                    super::super::assistant_access_policy::require_same_live_turn(
                        &prepared.chat,
                        &live,
                    )?;
                    let (mut row, mut private) =
                        persistence::load(state, &prepared.root_id, Some(&mut *session)).await?;
                    persistence::require_row(&row, &prepared.chat)?;
                    if row.phase != "unactivated" {
                        return Err(conflict());
                    }
                    let control = private
                        .controls
                        .iter()
                        .find(|control| token_hash(control.token.expose()) == prepared.control_hash)
                        .ok_or_else(invalid)?;
                    if control.claims.protocol != wire::PROTOCOL
                        || control.claims.exp <= Utc::now().timestamp() as u64
                        || Some(control.claims.active_turn_id.as_str())
                            != prepared.chat.turn_id.as_deref()
                    {
                        return Err(invalid());
                    }
                    let binding = super::super::assistant_service_execution::connection_binding(
                        &state.db,
                        &prepared.connection_id,
                        &prepared.chat.api_key_id,
                    )
                    .await?;
                    if binding != prepared.connection_binding {
                        return Err(conflict());
                    }
                    // A concurrent callback may have durably disabled observation.
                    // That is not a new denial of an otherwise admitted ordinary Read.
                    if !private.preconsent.ineligible {
                        let read = private
                            .preconsent
                            .reads
                            .iter_mut()
                            .find(|r| r.record.id == prepared.step_id)
                            .ok_or_else(conflict)?;
                        if read.control_hash != prepared.control_hash
                            || read.record.request_binding != prepared.request_binding
                            || !matches!(read.record.phase, StepPhase::Prepared)
                        {
                            return Err(conflict());
                        }
                        read.record.phase = StepPhase::Dispatched {
                            dispatch_id: dispatch_id.clone(),
                        };
                        persistence::save(state, &mut row, &private, session).await?;
                    }
                    Ok(())
                }
                .await;
                super::super::api_key_mutation_service::transaction_result(result)
            })
        })
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)?;
    Ok(ClaimedDispatch {
        root_id: prepared.root_id.clone(),
        step_id: prepared.step_id.clone(),
        dispatch_id,
        operation: StepOperation::Generation,
        read_observation: true,
    })
}

pub(super) fn status(
    private: &RootPrivate,
    binding: Option<&str>,
    control_hash: &str,
) -> wire::PrepareReply {
    if private.preconsent.ineligible {
        return wire::PrepareReply::ReadIneligible;
    }
    let Some(read) = private.preconsent.reads.last() else {
        return wire::PrepareReply::ReadInProgress;
    };
    if binding != Some(read.record.request_binding.as_str()) || control_hash != read.control_hash {
        return wire::PrepareReply::ReadIneligible;
    }
    match &read.record.phase {
        StepPhase::Completed { receipt, .. } => wire::PrepareReply::ReadCompleted {
            receipt: receipt.clone(),
        },
        StepPhase::Prepared | StepPhase::Dispatched { .. } => wire::PrepareReply::ReadInProgress,
        StepPhase::OutcomeUnknown { .. } => wire::PrepareReply::ReadIneligible,
    }
}
