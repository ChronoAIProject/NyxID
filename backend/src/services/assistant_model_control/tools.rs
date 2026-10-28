//! Correlate observed provider declarations with actual authenticated MCP
//! execution. JSON-RPC IDs and bridge-supplied result text are never evidence.
use super::*;

pub(crate) struct ToolDispatch {
    root_id: String,
    step_id: String,
    call_id: String,
    dispatch_id: String,
}

fn declaration<'a>(
    step: &'a Step,
    name: &str,
    arguments: &Value,
) -> AppResult<&'a ProviderToolCall> {
    let StepPhase::Completed { completion, .. } = &step.phase else {
        return Err(conflict());
    };
    let mut matching = completion.calls.iter().filter(|call| {
        call.name == name
            && serde_json::from_str::<Value>(&call.arguments).ok().as_ref() == Some(arguments)
    });
    let call = matching.next().ok_or_else(conflict)?;
    // The native tool event has no trusted provider call_id. Duplicate equal
    // declarations are ambiguous, so neither one can be selected by guessing.
    if matching.next().is_some() {
        return Err(conflict());
    }
    Ok(call)
}

pub(crate) async fn tool_prepare(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    request: wire::ToolPrepareRequest,
) -> AppResult<wire::PreparedTool> {
    if request.protocol != wire::PROTOCOL
        || request.name.len() > 256
        || !request.arguments.is_object()
        || serde_json::to_vec(&request.arguments)
            .map_err(|_| invalid())?
            .len()
            > wire::MAX_ITEM_BYTES
    {
        return Err(invalid());
    }
    let (root, _, _) = persistence::authenticated(state, chat, token, None).await?;
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async |session| {
            let result = async {
                let (live, _, _) = acks::fence(&state.db, chat, session).await?;
                super::super::assistant_access_policy::require_same_live_turn(chat, &live)?;
                let (mut row, mut private) =
                    persistence::load(state, &root.id, Some(&mut *session)).await?;
                persistence::require_row(&row, chat)?;
                if row.phase != "allowed" {
                    return Err(conflict());
                }
                let step = private.steps.last_mut().ok_or_else(conflict)?;
                let call_id = declaration(step, &request.name, &request.arguments)?
                    .call_id
                    .clone();
                let capability =
                    if let Some(tool) = step.tools.iter().find(|t| t.call_id == call_id) {
                        if !matches!(tool.phase, HostToolPhase::Prepared) {
                            return Err(conflict());
                        }
                        tool.capability.clone()
                    } else {
                        let capability = persistence::new_token()?;
                        step.tools.push(HostTool {
                            call_id: call_id.clone(),
                            capability: capability.clone(),
                            phase: HostToolPhase::Prepared,
                        });
                        capability
                    };
                let reply = wire::PreparedTool {
                    root_id: row.id.clone(),
                    step_id: step.id.clone(),
                    call_id,
                    tool_bearer: capability,
                };
                persistence::save(state, &mut row, &private, session).await?;
                Ok(reply)
            }
            .await;
            super::super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn tool_status(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    request: wire::ToolStatusRequest,
) -> AppResult<wire::ToolResultReceipt> {
    if request.protocol != wire::PROTOCOL {
        return Err(invalid());
    }
    let (row, private, _) = persistence::authenticated(state, chat, token, None).await?;
    if row.id != request.tool.root_id {
        return Err(invalid());
    }
    let tool = private
        .steps
        .iter()
        .find(|s| s.id == request.tool.step_id)
        .and_then(|s| {
            s.tools.iter().find(|t| {
                t.call_id == request.tool.call_id
                    && token_hash(t.capability.expose())
                        == token_hash(request.tool.tool_bearer.expose())
            })
        })
        .ok_or_else(invalid)?;
    match &tool.phase {
        HostToolPhase::Completed { receipt } => Ok(receipt.clone()),
        _ => Err(conflict()),
    }
}

pub(crate) async fn tool_claim(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    capability: &wire::SecretToken,
    name: &str,
    arguments: &Value,
) -> AppResult<ToolDispatch> {
    let (root, _, _) = persistence::authenticated(state, chat, token, None).await?;
    let dispatch_id = uuid::Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    let (step_id, call_id) = session
        .start_transaction()
        .and_run2(async |session| {
            let result = async {
                let (live, _, _) = acks::fence(&state.db, chat, session).await?;
                super::super::assistant_access_policy::require_same_live_turn(chat, &live)?;
                let (mut row, mut private) =
                    persistence::load(state, &root.id, Some(&mut *session)).await?;
                persistence::require_row(&row, chat)?;
                if row.phase != "allowed" {
                    return Err(conflict());
                }
                let step = private.steps.last_mut().ok_or_else(conflict)?;
                let call_id = declaration(step, name, arguments)?.call_id.clone();
                let tool = step
                    .tools
                    .iter_mut()
                    .find(|t| {
                        t.call_id == call_id
                            && token_hash(t.capability.expose()) == token_hash(capability.expose())
                    })
                    .ok_or_else(invalid)?;
                if !matches!(tool.phase, HostToolPhase::Prepared) {
                    return Err(conflict());
                }
                tool.phase = HostToolPhase::Dispatched {
                    dispatch_id: dispatch_id.clone(),
                };
                let ids = (step.id.clone(), call_id);
                persistence::save(state, &mut row, &private, session).await?;
                Ok(ids)
            }
            .await;
            super::super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)?;
    Ok(ToolDispatch {
        root_id: root.id,
        step_id,
        call_id,
        dispatch_id,
    })
}

/// Runs after the actual handler, with its genuine saved activity ID. The
/// first contract supports successful, untransformed JSON text only. Existing
/// refusal guidance/truncation still runs, but cannot mint a reusable receipt.
pub(crate) async fn tool_complete(
    state: &AppState,
    dispatch: ToolDispatch,
    activity_id: Option<&str>,
    result: Option<&Value>,
) -> AppResult<()> {
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run2(async |session| {
        let result=async {
            let (mut row,mut private)=persistence::load(state,&dispatch.root_id,Some(&mut *session)).await?;
            let step=private.steps.iter_mut().find(|s| s.id==dispatch.step_id).ok_or_else(conflict)?;
            let StepPhase::Completed {completion,..}=&step.phase else {return Err(conflict());};
            let call=completion.calls.iter().find(|c| c.call_id==dispatch.call_id).ok_or_else(conflict)?;
            let tool=step.tools.iter_mut().find(|t| t.call_id==dispatch.call_id).ok_or_else(conflict)?;
            if !matches!(&tool.phase,HostToolPhase::Dispatched {dispatch_id} if dispatch_id==&dispatch.dispatch_id)
            {return Err(conflict());}
            tool.phase=match (activity_id,result) {
                (Some(activity),Some(result)) if !activity.is_empty() && result.is_object()
                    && result["isError"]!=true && result.to_string().len()<=wire::MAX_ITEM_BYTES=>{
                    HostToolPhase::Completed {receipt:wire::ToolResultReceipt {
                        call_id:call.call_id.clone(),tool_name:call.name.clone(),arguments_binding:digest(json!(call.arguments)),
                        result_binding:digest(json!(result.to_string())),server_activity_id:activity.into(),bearer:persistence::new_token()?,
                    }}
                }
                _=>HostToolPhase::Unsupported,
            };
            persistence::save(state,&mut row,&private,session).await
        }.await;
        super::super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
}
