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
    call_id: Option<&str>,
    name: &str,
    arguments: &Value,
) -> AppResult<&'a ProviderToolCall> {
    let StepPhase::Completed { completion, .. } = &step.phase else {
        return Err(conflict());
    };
    let mut matching = completion.calls.iter().filter(|call| {
        call_id.is_none_or(|id| id == call.call_id)
            && call.name == name
            && serde_json::from_str::<Value>(&call.arguments).ok().as_ref() == Some(arguments)
    });
    let call = matching.next().ok_or_else(conflict)?;
    // Older native events do not carry a call_id. Only that compatibility
    // branch needs unique name/arguments; a witnessed callId selects exactly
    // its provider-observed declaration, never a JSON-RPC message ID.
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
        || request.call_id.as_ref().is_some_and(|id| {
            id.is_empty() || id.len() > 256 || !id.bytes().all(|b| b.is_ascii_graphic())
        })
        || !(1024..=wire::MAX_TOOL_RESULT_CHARS).contains(&request.result_char_limit)
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
                let control_hash = token_hash(token.expose());
                let source = match request.refusal_source.as_ref() {
                    Some(receipt) => Some(refusal_source(
                        &private,
                        receipt,
                        &control_hash,
                        request.result_char_limit,
                    )?),
                    None => None,
                };
                if let Some((_,evidence))=source.as_ref() {
                    require_refusal_reuse(&private,evidence,&control_hash,request.result_char_limit,&request.name,&request.arguments)?;
                }
                let step = private.steps.last_mut().ok_or_else(conflict)?;
                let call = declaration(
                    step,
                    request.call_id.as_deref(),
                    &request.name,
                    &request.arguments,
                )?
                .clone();
                let call_id = call.call_id.clone();
                let source_hash = request
                    .refusal_source
                    .as_ref()
                    .map(|r| token_hash(r.bearer.expose()));
                let capability =
                    if let Some(tool) = step.tools.iter().find(|t| t.call_id == call_id) {
                        if tool.result_char_limit != request.result_char_limit
                            || tool.control_hash.as_deref() != Some(control_hash.as_str())
                            || (tool.refusal_source_hash != source_hash
                                && !matches!((&tool.phase,source_hash.as_deref()),(HostToolPhase::Completed{receipt},Some(hash)) if token_hash(receipt.bearer.expose())==hash))
                            || !(matches!(tool.phase, HostToolPhase::Prepared)
                                || (source_hash.is_some()
                                    && matches!(tool.phase, HostToolPhase::Completed { .. })))
                        {
                            return Err(conflict());
                        }
                        tool.capability.clone()
                    } else {
                        let capability = persistence::new_token()?;
                        let (phase, evidence) = if let Some((original, evidence)) = source {
                            let receipt = wire::ToolResultReceipt {
                                step_id: step.id.clone(),
                                call_id: call.call_id.clone(),
                                tool_name: call.name.clone(),
                                arguments_binding: digest(json!(call.arguments)),
                                result_binding: original.result_binding,
                                success:original.success,
                                server_activity_id: original.server_activity_id,
                                evidence: wire::ToolResultEvidence::ReusedRefusal {
                                    origin_step_id: evidence.origin_step_id.clone(),
                                    origin_call_id: evidence.origin_call_id.clone(),
                                },
                                bearer: persistence::new_token()?,
                            };
                            (HostToolPhase::Completed { receipt }, Some(evidence))
                        } else {
                            (HostToolPhase::Prepared, None)
                        };
                        step.tools.push(HostTool {
                            call_id: call_id.clone(),
                            capability: capability.clone(),
                            result_char_limit: request.result_char_limit,
                            control_hash: Some(control_hash),
                            refusal_evidence: evidence,
                            refusal_source_hash: source_hash,
                            phase,
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
    if row.id != request.tool.root_id || row.phase != "allowed" {
        return Err(invalid());
    }
    let tool = private
        .steps
        .iter()
        .find(|s| s.id == request.tool.step_id)
        .and_then(|s| {
            s.tools.iter().find(|t| {
                t.call_id == request.tool.call_id
                    && t.control_hash.as_deref() == Some(token_hash(token.expose()).as_str())
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
                let call_id = step
                    .tools
                    .iter()
                    .find(|t| token_hash(t.capability.expose()) == token_hash(capability.expose()))
                    .map(|t| t.call_id.clone())
                    .ok_or_else(invalid)?;
                declaration(step, Some(&call_id), name, arguments)?;
                let tool = step
                    .tools
                    .iter_mut()
                    .find(|t| {
                        t.call_id == call_id
                            && token_hash(t.capability.expose()) == token_hash(capability.expose())
                    })
                    .ok_or_else(invalid)?;
                if tool.control_hash.as_deref() != Some(token_hash(token.expose()).as_str())
                    || !matches!(tool.phase, HostToolPhase::Prepared)
                {
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
/// result is projected using the first-prepare-bound character limit and the
/// exact existing NyxAgent projection, including refusal guidance/truncation.
/// An activity ID proves actual handler execution, not absence of prior sends.
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
                    && result.to_string().len()<=wire::MAX_TOOL_RESULT_BYTES=>{
                    let projection=wire::tool_result_projection(result,tool.result_char_limit);
                    tool.refusal_evidence=projection.refusal.as_ref().and_then(|refusal| {
                        let args=serde_json::from_str::<Value>(&call.arguments).ok()?;
                        let (target_name,target_arguments_hash,attempt_hash,_)=refusal_attempt(&call.name,&args)?;
                        Some(RefusalEvidence{kind:refusal.kind,target_name,target_arguments_hash,attempt_hash,
                            origin_step_id:step.id.clone(),origin_call_id:call.call_id.clone()})
                    });
                    HostToolPhase::Completed {receipt:wire::ToolResultReceipt {
                        step_id:step.id.clone(),
                        call_id:call.call_id.clone(),tool_name:call.name.clone(),arguments_binding:digest(json!(call.arguments)),
                        result_binding:digest(json!(projection.text)),success:projection.success,server_activity_id:activity.into(),evidence:wire::ToolResultEvidence::Executed,bearer:persistence::new_token()?,
                    }}
                }
                _=>HostToolPhase::Unsupported,
            };
            persistence::save(state,&mut row,&private,session).await
        }.await;
        super::super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
}

/// Same canonical target/ack rules as TurnTools::Attempt. No raw result or
/// arguments are retained in refusal evidence; they remain provider-bound.
fn refusal_attempt(name: &str, args: &Value) -> Option<(String, String, String, bool)> {
    let (name, mut args) = if name == "nyx__call_tool" {
        (
            args["tool_name"].as_str()?,
            serde_json::from_str::<Value>(args["arguments_json"].as_str()?).ok()?,
        )
    } else {
        (name, args.clone())
    };
    if !args.is_object() {
        return None;
    }
    args.sort_all_objects();
    let attempt = token_hash(&args.to_string());
    let has_ack = args["acknowledgement_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty());
    args.as_object_mut()?.remove("acknowledgement_id");
    Some((name.into(), token_hash(&args.to_string()), attempt, has_ack))
}

fn refusal_source(
    private: &RootPrivate,
    receipt: &wire::ToolResultReceipt,
    control_hash: &str,
    limit: usize,
) -> AppResult<(wire::ToolResultReceipt, RefusalEvidence)> {
    for step in &private.steps {
        for tool in &step.tools {
            if tool.control_hash.as_deref() != Some(control_hash) || tool.result_char_limit != limit
            {
                continue;
            }
            if let (HostToolPhase::Completed { receipt: saved }, Some(evidence)) =
                (&tool.phase, &tool.refusal_evidence)
                && saved.step_id == receipt.step_id
                && saved.call_id == receipt.call_id
                && saved.tool_name == receipt.tool_name
                && saved.arguments_binding == receipt.arguments_binding
                && saved.result_binding == receipt.result_binding
                && saved.success == receipt.success
                && saved.server_activity_id == receipt.server_activity_id
                && saved.evidence == receipt.evidence
                && token_hash(saved.bearer.expose()) == token_hash(receipt.bearer.expose())
            {
                return Ok((saved.clone(), evidence.clone()));
            }
        }
    }
    Err(conflict())
}

fn require_refusal_reuse(
    private: &RootPrivate,
    evidence: &RefusalEvidence,
    control_hash: &str,
    limit: usize,
    name: &str,
    args: &Value,
) -> AppResult<()> {
    let attempt = refusal_attempt(name, args).ok_or_else(conflict)?;
    if attempt.0 != evidence.target_name || attempt.1 != evidence.target_arguments_hash {
        return Err(conflict());
    }
    if evidence.kind == wire::RefusalKind::Invalid
        && attempt.3
        && attempt.2 != evidence.attempt_hash
    {
        // Local Invalid(A), Invalid(B), repeat A returns the latest B text.
        // Prove A's previous *actual* invalid attempt independently; B remains
        // the canonical source of text/activity. Reuse alone cannot invent A.
        let proved=private.steps.iter().flat_map(|s|s.tools.iter()).any(|tool|
            tool.control_hash.as_deref()==Some(control_hash) && tool.result_char_limit==limit
            && matches!(&tool.phase,HostToolPhase::Completed{receipt} if receipt.evidence==wire::ToolResultEvidence::Executed)
            && tool.refusal_evidence.as_ref().is_some_and(|old|old.kind==wire::RefusalKind::Invalid
                && old.target_name==attempt.0 && old.target_arguments_hash==attempt.1 && old.attempt_hash==attempt.2));
        if !proved {
            return Err(conflict());
        }
    }
    Ok(())
}

#[cfg(test)]
mod reuse_tests {
    use super::*;

    fn token(text: &str) -> wire::SecretToken {
        wire::SecretToken::parse(text.into()).unwrap()
    }
    fn completed_invalid(call_id: &str, ack: &str) -> Step {
        let args = json!({"tool_name":"effect__write","arguments_json":json!({"x":1,"acknowledgement_id":ack}).to_string()});
        let (target_name, target_arguments_hash, attempt_hash, _) =
            refusal_attempt("nyx__call_tool", &args).unwrap();
        Step {
            id: format!("step-{call_id}"),
            ordinal: 0,
            request_binding: "request".into(),
            input: vec![],
            capability_hash: "step".into(),
            capability_ciphertext: vec![],
            phase: StepPhase::Completed {
                completion: ObservedCompletion {
                    response_id: "response".into(),
                    output: vec![],
                    calls: vec![],
                },
                receipt: wire::CompletionReceipt {
                    root_id: "root".into(),
                    step_id: format!("step-{call_id}"),
                    ordinal: 0,
                    request_binding: "request".into(),
                    output_binding: "output".into(),
                    receipt_bearer: token("completion"),
                },
            },
            tools: vec![HostTool {
                call_id: call_id.into(),
                capability: token("tool"),
                result_char_limit: 1024,
                control_hash: Some("control".into()),
                refusal_source_hash: None,
                refusal_evidence: Some(RefusalEvidence {
                    kind: wire::RefusalKind::Invalid,
                    target_name,
                    target_arguments_hash,
                    attempt_hash,
                    origin_step_id: format!("step-{call_id}"),
                    origin_call_id: call_id.into(),
                }),
                phase: HostToolPhase::Completed {
                    receipt: wire::ToolResultReceipt {
                        step_id: format!("step-{call_id}"),
                        call_id: call_id.into(),
                        tool_name: "nyx__call_tool".into(),
                        arguments_binding: digest(json!(args.to_string())),
                        result_binding: format!("latest-text-{ack}"),
                        success: true,
                        server_activity_id: format!("activity-{call_id}"),
                        evidence: wire::ToolResultEvidence::Executed,
                        bearer: token(&format!("receipt-{call_id}")),
                    },
                },
            }],
        }
    }

    #[test]
    fn invalid_a_b_a_reuses_latest_b_only_with_prior_actual_a_and_same_control() {
        // Server state-unit proof; genuine activity is covered by real MCP tests.
        let mut private = RootPrivate {
            controls: vec![],
            envelope: None,
            initial_input: vec![],
            connection_id: None,
            connection_binding: None,
            owner_id: None,
            catalog_id: None,
            card_arguments: None,
            effective_headers_binding: None,
            steps: vec![
                completed_invalid("call-a", "A"),
                completed_invalid("call-b", "B"),
            ],
        };
        let source = match &private.steps[1].tools[0].phase {
            HostToolPhase::Completed { receipt } => receipt.clone(),
            _ => unreachable!(),
        };
        let (_, evidence) = refusal_source(&private, &source, "control", 1024).unwrap();
        let arguments = |ack: &str| json!({"tool_name":"effect__write","arguments_json":json!({"x":1,"acknowledgement_id":ack}).to_string()});
        require_refusal_reuse(
            &private,
            &evidence,
            "control",
            1024,
            "nyx__call_tool",
            &arguments("A"),
        )
        .unwrap();
        require_refusal_reuse(
            &private,
            &evidence,
            "control",
            1024,
            "nyx__call_tool",
            &arguments(""),
        )
        .unwrap();
        assert_eq!(source.result_binding, "latest-text-B");
        assert_eq!(evidence.origin_call_id, "call-b");
        assert!(
            require_refusal_reuse(
                &private,
                &evidence,
                "control",
                1024,
                "nyx__call_tool",
                &arguments("never-observed")
            )
            .is_err()
        );
        assert!(refusal_source(&private, &source, "other-control", 1024).is_err());
        assert!(refusal_source(&private, &source, "control", 2048).is_err());
        let mut forged = source.clone();
        forged.evidence = wire::ToolResultEvidence::ReusedRefusal {
            origin_step_id: "forged".into(),
            origin_call_id: "call-b".into(),
        };
        assert!(refusal_source(&private, &forged, "control", 1024).is_err());
        let mut changed = arguments("A");
        changed["tool_name"] = json!("other__write");
        assert!(
            require_refusal_reuse(
                &private,
                &evidence,
                "control",
                1024,
                "nyx__call_tool",
                &changed
            )
            .is_err()
        );
        // A reused result cannot manufacture proof of an earlier actual attempt.
        if let HostToolPhase::Completed { receipt } = &mut private.steps[0].tools[0].phase {
            receipt.evidence = wire::ToolResultEvidence::ReusedRefusal {
                origin_step_id: "some".into(),
                origin_call_id: "some".into(),
            };
        }
        assert!(
            require_refusal_reuse(
                &private,
                &evidence,
                "control",
                1024,
                "nyx__call_tool",
                &arguments("A")
            )
            .is_err()
        );
    }
}
