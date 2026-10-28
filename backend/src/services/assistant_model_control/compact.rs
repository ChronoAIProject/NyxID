//! Canonical standalone compaction, separate from native history projection.
//! This module never turns retained function calls into executable declarations.
use super::*;

/// Sealed with the generation envelope before the owner sees the first card.
/// Generation-only tools/options remain fixed in that envelope. The compact
/// endpoint has its own closed configuration; it cannot learn new tools later.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct Configuration {
    fixed: Value,
}

pub(super) fn seal(generation: &Value) -> Configuration {
    let mut fixed = json!({"model": generation["model"]});
    if let Some(instructions) = generation.get("instructions") {
        fixed["instructions"] = instructions.clone();
    }
    Configuration { fixed }
}

pub(super) fn input(
    configuration: &Configuration,
    candidate: &wire::PrepareRequest,
) -> AppResult<Vec<Value>> {
    if candidate.protocol != wire::PROTOCOL
        || candidate.relative_path != "responses/compact"
        || candidate.query.is_some()
        || serde_json::to_vec(&candidate.body)
            .map_err(|_| unsupported())?
            .len()
            > wire::MAX_PREPARE_BYTES
    {
        return Err(unsupported());
    }
    let mut fixed = candidate.body.clone();
    let object = fixed.as_object_mut().ok_or_else(unsupported)?;
    let input = object
        .remove("input")
        .and_then(|v| v.as_array().cloned())
        .ok_or_else(unsupported)?;
    // Equality includes omission and exact privileged instructions. Unknown
    // compact options, generation tools, metadata and store/stream are refused.
    if fixed != configuration.fixed {
        return Err(unsupported());
    }
    Ok(input)
}

pub(super) struct VerifiedCompaction {
    pub response_id: String,
    pub output: Vec<Value>,
    pub response: Value,
}

/// Invoked only after successful JSON transport reaches real upstream EOF.
/// The official standalone contract returns object=response.compaction and
/// requires the complete output as the next input, without pruning. A native
/// filter/reinjection implementation is deliberately not a second comparison.
pub(super) fn verify(value: Value, sent: &[Value]) -> AppResult<VerifiedCompaction> {
    let object = value.as_object().ok_or_else(unsupported)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "id" | "object" | "created_at" | "output" | "usage"
        )
    }) || value["object"] != "response.compaction"
        || value["created_at"].as_u64().is_none()
        || !value["usage"].is_object()
    {
        return Err(unsupported());
    }
    let response_id = value["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or_else(unsupported)?
        .to_owned();
    let output = value["output"]
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= wire::MAX_ITEMS)
        .ok_or_else(unsupported)?
        .clone();
    if serde_json::to_vec(&value).map_err(|_| unsupported())?.len() > wire::MAX_OUTPUT_BYTES {
        return Err(unsupported());
    }
    let mut compactions = 0;
    let mut retained = 0;
    let mut identifiers = std::collections::BTreeSet::new();
    for item in &output {
        if serde_json::to_vec(item).map_err(|_| unsupported())?.len() > wire::MAX_ITEM_BYTES {
            return Err(unsupported());
        }
        if let Some(id) = item.get("id").and_then(Value::as_str)
            && !identifiers.insert(id)
        {
            return Err(unsupported());
        }
        if item["type"] == "compaction" {
            validate_item(item)?;
            compactions += 1;
        } else if matches!(item["role"].as_str(), Some("system" | "developer"))
            || matches!(
                item["type"].as_str(),
                Some("function_call" | "function_call_output")
            )
        {
            envelope::validate_items(std::slice::from_ref(item))?;
            // Privileged text and host-tool records may only be retained from
            // the actual sent window, in order. The provider may serialize
            // completed status/item identity; all payload fields still match.
            // The complete returned item is retained and bound as-is. It does
            // not become a new executable declaration or tool receipt.
            let identity = history_identity(item);
            let position = sent[retained..]
                .iter()
                .position(|prior| history_identity(prior) == identity)
                .ok_or_else(unsupported)?;
            retained += position + 1;
        } else {
            // Compaction is allowed to synthesize closed nonprivileged text
            // and reasoning. Do not invent an equality rule for every retained
            // message when the provider contract returns a canonical window.
            envelope::validate_items(std::slice::from_ref(item))?;
        }
    }
    if compactions == 0 {
        return Err(unsupported());
    }
    Ok(VerifiedCompaction {
        response_id,
        output,
        response: value,
    })
}

fn history_identity(item: &Value) -> Value {
    let mut identity = item.clone();
    if let Some(object) = identity.as_object_mut() {
        object.remove("id");
        object.remove("status");
    }
    identity
}

pub(super) fn validate_item(item: &Value) -> AppResult<()> {
    if item.as_object().is_none_or(|object| {
        object
            .keys()
            .any(|key| !matches!(key.as_str(), "id" | "type" | "encrypted_content"))
    }) || item["type"] != "compaction"
        || item["id"]
            .as_str()
            .is_none_or(|id| id.is_empty() || id.len() > 256)
        || item["encrypted_content"]
            .as_str()
            .is_none_or(|text| text.is_empty() || text.len() > wire::MAX_ITEM_BYTES)
    {
        return Err(unsupported());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact_response(output: Value) -> Value {
        json!({"id":"cmp-1","object":"response.compaction","created_at":1793160000,
            "output":output,"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}})
    }

    #[test]
    fn canonical_compact_configuration_is_sealed_before_allow_without_learning_generation_options()
    {
        let fixed = json!({"model":"fixed-model","instructions":"fixed privileged instructions",
            "tools":[{"type":"function","name":"nyx__search_tools","parameters":{}}],
            "client_metadata":{"unknown":"opaque"},"store":false,"stream":true});
        let configuration = seal(&fixed);
        let candidate = wire::PrepareRequest {
            protocol: wire::PROTOCOL.into(),
            relative_path: "responses/compact".into(),
            query: None,
            semantic_headers: vec![("content-type".into(), "application/json".into())],
            body: json!({"model":"fixed-model","instructions":"fixed privileged instructions","input":[]}),
            preceding_receipt: None,
            tool_results: vec![],
        };
        assert!(input(&configuration, &candidate).is_ok());
        for (key, value) in [
            ("model", json!("changed-model")),
            ("instructions", json!("new privileged instructions")),
            ("tools", fixed["tools"].clone()),
            ("client_metadata", fixed["client_metadata"].clone()),
            ("store", json!(false)),
            ("stream", json!(false)),
            ("unknown", json!("unreviewed")),
        ] {
            let mut changed: wire::PrepareRequest =
                serde_json::from_value(serde_json::to_value(&candidate).unwrap()).unwrap();
            changed.body[key] = value;
            assert!(input(&configuration, &changed).is_err(), "{key}");
        }
        let mut omitted = candidate;
        omitted.body.as_object_mut().unwrap().remove("instructions");
        assert!(input(&configuration, &omitted).is_err());
    }

    #[test]
    fn canonical_compact_family_cannot_be_injected_initially_or_added_to_saved_absent_envelope() {
        let generation = wire::PrepareRequest {
            protocol: wire::PROTOCOL.into(),
            relative_path: "responses".into(),
            query: None,
            semantic_headers: vec![("content-type".into(), "application/json".into())],
            body: json!({"model":"fixed-model","store":false,"input":[{"role":"user","content":"question"}]}),
            preceding_receipt: None,
            tool_results: vec![],
        };
        let (mut saved, _) = envelope::envelope(&generation).unwrap();
        assert!(saved.compact.is_some());
        let compact = wire::PrepareRequest {
            protocol: wire::PROTOCOL.into(),
            relative_path: "responses/compact".into(),
            query: None,
            semantic_headers: generation.semantic_headers.clone(),
            body: json!({"model":"fixed-model","input":generation.body["input"]}),
            preceding_receipt: None,
            tool_results: vec![],
        };
        assert!(envelope::within_family(&saved, &compact).is_ok());
        saved.compact = None;
        assert!(envelope::within_family(&saved, &generation).is_ok());
        assert!(envelope::within_family(&saved, &compact).is_err());
        let mut injected = generation;
        injected.body["input"].as_array_mut().unwrap().push(
            json!({"type":"compaction","id":"forged-initial","encrypted_content":"unwitnessed"}),
        );
        assert!(envelope::envelope(&injected).is_err());
        assert!(envelope::within_family(&saved, &injected).is_err());
    }

    #[test]
    fn canonical_compact_output_binds_whole_window_and_never_invents_privilege_or_tool_history() {
        let developer = json!({"role":"developer","content":"approved instructions"});
        let call = json!({"type":"function_call","call_id":"historical-call","name":"nyx__search_tools","arguments":"{}"});
        let result = json!({"type":"function_call_output","call_id":"historical-call","output":"real historical result"});
        let sent = vec![developer.clone(), call.clone(), result.clone()];
        let output = json!([developer, call, result,
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"provider compact summary"}]},
            {"type":"compaction","id":"cmp-item-1","encrypted_content":"opaque-provider-ciphertext"}]);
        let verified = verify(compact_response(output.clone()), &sent).unwrap();
        assert_eq!(verified.output, output.as_array().unwrap().clone());
        assert_eq!(verified.response["output"], output);
        let mut assigned_identity = output.clone();
        assigned_identity[1]["id"] = json!("provider-retained-call-id");
        assigned_identity[1]["status"] = json!("completed");
        let assigned = verify(compact_response(assigned_identity.clone()), &sent).unwrap();
        assert_eq!(assigned.response["output"], assigned_identity);
        for (index, key, value) in [
            (0, "content", json!("invented privileged instructions")),
            (1, "call_id", json!("invented-call")),
            (1, "arguments", json!("{\"changed\":true}")),
            (1, "status", json!("in_progress")),
            (1, "id", json!({"malformed":"identity"})),
            (2, "output", json!("invented host result")),
            (4, "encrypted_content", Value::Null),
            (4, "unknown", json!("unreviewed opaque shape")),
        ] {
            let mut changed = output.clone();
            changed[index][key] = value;
            assert!(verify(compact_response(changed), &sent).is_err(), "{key}");
        }
        let mut reordered = output;
        reordered.as_array_mut().unwrap().swap(1, 2);
        assert!(verify(compact_response(reordered), &sent).is_err());
        assert!(verify(compact_response(json!([sent[0].clone()])), &sent).is_err());
    }
}
