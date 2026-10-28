//! Initial compact consent. A prospective configuration is requested authority;
//! it cannot replace known Read output or certify historical tool effects.
use super::*;

fn proposal(candidate: &wire::PrepareRequest, fixed: &Value) -> AppResult<(envelope::Envelope, Vec<Value>)> {
    let mut body = fixed.clone();
    let object = body.as_object_mut().ok_or_else(unsupported)?;
    if object.contains_key("input") { return Err(unsupported()); }
    object.insert("input".into(), candidate.body["input"].clone());
    let generation = wire::PrepareRequest {
        protocol: candidate.protocol.clone(), relative_path: "responses".into(), query: None,
        semantic_headers: candidate.semantic_headers.clone(), body, generation_proposal: None,
        preceding_receipt: None, tool_results: vec![],
    };
    let (envelope, input) = envelope::envelope(&generation)?;
    let actual = compact::input(envelope.compact.as_ref().ok_or_else(unsupported)?, candidate)?;
    if input != actual { return Err(conflict()); }
    Ok((envelope, input))
}

pub(super) fn candidate(
    private: &RootPrivate,
    token: &wire::SecretToken,
    candidate: &wire::PrepareRequest,
) -> AppResult<read_observation::InitialCompact> {
    if private.preconsent.ineligible { return Err(unsupported()); }
    let (envelope, input, read_id) = if private.preconsent.seen {
        // An explicit proposal may not relabel known calls/results as a new
        // initial snapshot. Reconcile the actual completed Read and receipts.
        let read = private.preconsent.reads.last().ok_or_else(conflict)?;
        let input = compact::input(read.envelope.compact.as_ref().ok_or_else(unsupported)?, candidate)?;
        let (_, checked) = envelope::within_family(&read.envelope, candidate)?;
        if input != checked { return Err(conflict()); }
        if candidate.generation_proposal.as_ref().is_some_and(|fixed| fixed != &read.envelope.fixed) {
            return Err(conflict());
        }
        read_observation::continuation(read, &input)?;
        let StepPhase::Completed { receipt, .. } = &read.record.phase else { return Err(conflict()); };
        if read.control_hash != token_hash(token.expose())
            || candidate.preceding_receipt.as_ref().is_none_or(|t| token_hash(t.expose()) != token_hash(receipt.receipt_bearer.expose())) {
            return Err(conflict());
        }
        let results = persistence::verify_tool_results(&read.record, &candidate.tool_results)?;
        let StepPhase::Completed { completion, .. } = &read.record.phase else { return Err(conflict()); };
        envelope::next_input(&read.record.input, completion, &input, &results)?;
        (read.envelope.clone(), input, Some(read.record.id.clone()))
    } else {
        if candidate.preceding_receipt.is_some() || !candidate.tool_results.is_empty() { return Err(conflict()); }
        let (envelope, input) = proposal(candidate, candidate.generation_proposal.as_ref().ok_or_else(unsupported)?)?;
        (envelope, input, None)
    };
    Ok(read_observation::InitialCompact {
        envelope, input, request_binding: envelope::request_binding(candidate),
        control_hash: token_hash(token.expose()), read_id,
    })
}

pub(super) async fn prepare(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    root: &ModelControlRoot,
    private: &RootPrivate,
    request: &wire::PrepareRequest,
) -> AppResult<wire::PrepareReply> {
    if private.preconsent.ineligible {
        return Ok(wire::PrepareReply::ExactOnly {reason:"Earlier Read history has no reusable observation proof. Existing exact approval applies.".into()});
    }
    if !private.preconsent.seen && request.generation_proposal.is_none() {
        return Ok(wire::PrepareReply::ExactOnly { reason: "Initial standalone compact requires a complete generation configuration before consent.".into() });
    }
    let proposed = candidate(private, token, request)?;
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run2(async |session| {
        let result = async {
            let (mut row, mut current) = persistence::load(state, &root.id, Some(&mut *session)).await?;
            persistence::require_row(&row, chat)?;
            if row.phase != "unactivated" || row.revision != root.revision { return Err(conflict()); }
            if let Some(saved) = &current.preconsent.compact {
                if saved.control_hash != proposed.control_hash || saved.request_binding != proposed.request_binding
                    || saved.envelope != proposed.envelope || saved.read_id != proposed.read_id { return Err(conflict()); }
            } else {
                current.preconsent.compact = Some(proposed.clone());
                persistence::save(state, &mut row, &current, session).await?;
            }
            Ok(wire::PrepareReply::Observe)
        }.await;
        super::super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
}

/// The actual resolved callback has no bridge-reported completion. Its exact
/// body must match the durable proposal, and known history is checked again.
pub(super) fn callback(
    private: &RootPrivate,
    token: &wire::SecretToken,
    actual: &wire::PrepareRequest,
    authority: &read_observation::Authority,
) -> AppResult<(envelope::Envelope, Vec<Value>)> {
    let saved = private.preconsent.compact.as_ref().ok_or_else(unsupported)?;
    if private.preconsent.ineligible || saved.control_hash != token_hash(token.expose())
        || saved.request_binding != envelope::request_binding(actual) { return Err(conflict()); }
    let (_, input) = envelope::within_family(&saved.envelope, actual)?;
    if saved.input != input { return Err(conflict()); }
    match &saved.read_id {
        Some(id) => {
            let read = private.preconsent.reads.last().filter(|r| &r.record.id == id && &r.authority == authority).ok_or_else(conflict)?;
            read_observation::continuation(read, &input)?;
        }
        None if private.preconsent.seen => return Err(conflict()),
        None => {}
    }
    Ok((saved.envelope.clone(), input))
}
