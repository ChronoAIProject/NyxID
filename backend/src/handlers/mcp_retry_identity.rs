//! Semantic Retry identity, separate from legacy exact-call/progress digests.
use serde_json::{Value, json};

/// This is also the dispatch decoder. A malformed preferred JSON string refuses
/// the call; a non-string keeps the established nested/flat fallback behavior.
pub(super) fn decode_arguments(wrapper: &Value) -> Result<Value, serde_json::Error> {
    if let Some(raw) = wrapper["arguments_json"].as_str() {
        serde_json::from_str(raw)
    } else if let Some(nested) = wrapper.get("arguments") {
        Ok(nested.clone())
    } else {
        let mut flat = wrapper.as_object().cloned().unwrap_or_default();
        for field in ["tool_name", "arguments_json", "result_query"] {
            flat.remove(field);
        }
        Ok(Value::Object(flat))
    }
}

fn business_call(params: &Value) -> Option<Value> {
    let name = params["name"].as_str()?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if name != "nyx__call_tool" {
        return Some(json!({"name":name,"arguments":args}));
    }
    let name = args["tool_name"]
        .as_str()
        .filter(|name| !name.is_empty() && name.len() <= 200)?;
    let query = crate::services::notion_evidence::validate_query(args.get("result_query")).ok()?;
    let inner = decode_arguments(&args).ok()?;
    let mut call = json!({"name":name,"arguments":inner});
    if let Some(query) = query {
        // This is wrapper projection metadata, distinct from a provider field
        // named result_query inside the decoded business arguments.
        call["result_query"] = json!(query);
    }
    Some(call)
}

pub(super) fn digest(params: &Value) -> String {
    let mut call = business_call(params).unwrap_or_else(|| params.clone());
    let field = match call["name"].as_str() {
        Some("nyxid__create_schedule") => Some("acknowledgement_id"),
        Some("nyxid__remember") => Some("recovery"),
        _ => None,
    };
    if let Some(field) = field
        && let Some(args) = call["arguments"].as_object_mut()
    {
        args.remove(field);
    }
    // Never run the legacy parser over provider argument values. Their types
    // and exact strings, even a field named arguments_json, are business input.
    format!(
        "v2:{}",
        crate::services::assistant_continuation::canonical_digest(&call)
    )
}

fn business_name(params: &Value) -> Option<&str> {
    let name = params["name"].as_str()?;
    let name = if name == "nyx__call_tool" {
        params["arguments"]["tool_name"]
            .as_str()
            .filter(|name| !name.is_empty() && name.len() <= 200)?
    } else {
        name
    };
    Some(name)
}

pub(super) fn provider_name(params: &Value) -> Option<&str> {
    let name = business_name(params)?;
    (!super::native_activity_names().contains(name)).then_some(name)
}

/// Preserve the base reader's normalized native evidence for every accepted
/// wrapper. Older readers consume retry_identity, not the additive v2 field.
pub(super) fn legacy_writer_digest(params: &Value) -> String {
    if matches!(
        business_name(params),
        Some("nyxid__create_schedule" | "nyxid__remember")
    ) && let Some(call) = business_call(params)
    {
        return super::legacy_chat_retry_digest(&call);
    }
    super::legacy_chat_retry_digest(params)
}

/// Historical generic labels cannot distinguish native writes either. Only
/// established native reads avoid that ambiguity gate; provider calls retain
/// their canonical name regardless of slug prefix or guessed HTTP semantics.
pub(super) fn review_tool(params: &Value) -> Option<&str> {
    if let Some(name) = provider_name(params) {
        return Some(name);
    }
    let name = business_name(params)?;
    let decoded;
    let args = if params["name"] == "nyx__call_tool" {
        decoded = match decode_arguments(&params["arguments"]) {
            Ok(args) => args,
            Err(_) => return Some(name),
        };
        &decoded
    } else {
        &params["arguments"]
    };
    (crate::services::assistant_acknowledgement_service::known_action_class(name, args)
        != Some(crate::services::assistant_acknowledgement_service::OwnerActionClass::Read))
    .then_some(name)
}

/// Can the legacy hash prove the decoded business identity? Its parser replaces
/// a top-level arguments_json string with JSON, including literal provider
/// fields. Every direct field value has another possible string preimage.
fn conclusive_exact(params: &Value) -> bool {
    let Some(raw) = params["arguments"].get("arguments_json") else {
        return true;
    };
    if params["name"] != "nyx__call_tool" || business_call(params).is_none() {
        return false;
    }
    let parsed = raw
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let normalized = parsed.as_ref().unwrap_or(raw);
    let identity = digest(params);
    // Both possible preimages must select the same business input: a JSON
    // string (decoded first), and its parsed value (nested/flat fallback).
    let mut alternate = params.clone();
    alternate["arguments"]["arguments_json"] = json!(normalized.to_string());
    if digest(&alternate) != identity {
        return false;
    }
    alternate["arguments"]["arguments_json"] = normalized.clone();
    // A string that parses again is not an unchanged preimage of this hash.
    super::chat_activity_digest(&alternate) != super::chat_activity_digest(params)
        || business_call(&alternate).is_none()
        || digest(&alternate) == identity
}

fn historical_representation(
    values: &mut crate::services::assistant_nyxagent::RetryDigestCandidates,
    params: &Value,
) {
    let exact = super::chat_activity_digest(params);
    let legacy = super::legacy_chat_retry_digest(params);
    if conclusive_exact(params) {
        values.conclusive.push(exact.clone());
    } else {
        values.lossy.push(exact.clone());
    }
    if legacy != exact {
        // Only the closed schedule/memory normalization creates a different
        // legacy digest. Classify its actual direct representation as well.
        let Some(mut normalized) = business_call(params) else {
            values.lossy.push(legacy);
            return;
        };
        let field = match normalized["name"].as_str() {
            Some("nyxid__create_schedule") => "acknowledgement_id",
            Some("nyxid__remember") => "recovery",
            _ => {
                values.lossy.push(legacy);
                return;
            }
        };
        if let Some(args) = normalized["arguments"].as_object_mut() {
            args.remove(field);
        }
        if conclusive_exact(&normalized) {
            values.conclusive.push(legacy);
        } else {
            values.lossy.push(legacy);
        }
    }
}

/// Historical matching retains the proof quality of every derivation. A hash
/// that erased provider types or wrapper precedence can only require review;
/// it must never create completed v2 evidence for the requested business call.
pub(super) fn matching_digests(
    params: &Value,
) -> crate::services::assistant_nyxagent::RetryDigestCandidates {
    let mut values = crate::services::assistant_nyxagent::RetryDigestCandidates {
        conclusive: vec![digest(params)],
        lossy: vec![],
    };
    historical_representation(&mut values, params);
    if let Some(call) = business_call(params) {
        let name = &call["name"];
        let args = &call["arguments"];
        if call.get("result_query").is_none() {
            historical_representation(&mut values, &json!({"name":name,"arguments":args}));
            if args == &json!({}) {
                historical_representation(&mut values, &json!({"name":name}));
            }
        }
        let mut wrappers = vec![
            json!({"tool_name":name,"arguments_json":args.to_string()}),
            json!({"tool_name":name,"arguments":args}),
        ];
        if let Some(fields) = args.as_object()
            && !["tool_name", "arguments_json", "arguments", "result_query"]
                .iter()
                .any(|key| fields.contains_key(*key))
        {
            let mut flat = args.clone();
            flat["tool_name"] = name.clone();
            wrappers.push(flat);
        }
        for mut wrapper in wrappers {
            if let Some(query) = call.get("result_query") {
                wrapper["result_query"] = query.clone();
            }
            historical_representation(
                &mut values,
                &json!({"name":"nyx__call_tool","arguments":wrapper}),
            );
        }
    }
    values.conclusive.sort();
    values.conclusive.dedup();
    values.lossy.sort();
    values.lossy.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_legacy_hash_derivations_distinguish_conclusive_and_lossy_inputs() {
        for value in [
            json!({"value":1}),
            json!([1]),
            json!(true),
            Value::Null,
            json!(1),
            json!("literal"),
            json!("\"literal\""),
            json!("{\"value\":1}"),
            json!(" { \"value\" : 1 } "),
        ] {
            let args = json!({"arguments_json":value});
            let direct = json!({"name":"nyx-vendor__write","arguments":args});
            let nested = json!({"name":"nyx__call_tool","arguments":{
                "tool_name":"nyx-vendor__write","arguments":args}});
            let historical = super::super::chat_activity_digest(&direct);
            let candidates = matching_digests(&nested);
            assert!(candidates.lossy.contains(&historical));
            assert!(!candidates.conclusive.contains(&historical));
            assert!(!conclusive_exact(&direct));
            assert!(conclusive_exact(&nested));
            let faithful = super::super::chat_activity_digest(&nested);
            assert!(candidates.conclusive.contains(&faithful));
        }
        let decoded = json!({"arguments_json":" { \"value\" : 1 } "});
        let fallback = json!({"arguments_json":{"value":2}});
        let json_form = json!({"name":"nyx__call_tool","arguments":{
            "tool_name":"nyx-vendor__write","arguments_json":decoded.to_string(),"arguments":fallback}});
        let mut object_form = json_form.clone();
        object_form["arguments"]["arguments_json"] = decoded.clone();
        assert_eq!(
            super::super::chat_activity_digest(&json_form),
            super::super::chat_activity_digest(&object_form)
        );
        assert_ne!(digest(&json_form), digest(&object_form));
        assert!(!conclusive_exact(&json_form));
        assert!(!conclusive_exact(&object_form));
        let mut safe = json_form.clone();
        safe["arguments"]["arguments"] = decoded;
        assert!(conclusive_exact(&safe));
        // A malformed unchanged string cannot be a completed alternate
        // preimage; JSON strings that decode scalar strings remain provable.
        for scalar in [json!("literal"), json!("{}"), json!(" { \"value\" : 1 } ")] {
            let wrapped = json!({"name":"nyx__call_tool","arguments":{
                "tool_name":"nyx-vendor__write","arguments_json":scalar.to_string()}});
            assert!(conclusive_exact(&wrapped));
        }
        for query in [Value::Null, json!({"invalid":true})] {
            let malformed = json!({"name":"nyx__call_tool","arguments":{
                "tool_name":"nyxid__create_schedule","arguments_json":"{}","result_query":query}});
            matching_digests(&malformed); // Malformed calls refuse without panics.
        }
    }

    #[test]
    fn retry_historical_review_exceptions_require_closed_reads() {
        for name in [
            "nyxid__run_schedule_now",
            "nyxid__remember",
            "nyxid__future_write",
            "nyx-provider__read",
        ] {
            assert_eq!(
                review_tool(&json!({"name":name,"arguments":{}})),
                Some(name)
            );
            assert_eq!(
                review_tool(&json!({"name":"nyx__call_tool","arguments":{
                "tool_name":name,"arguments_json":"{malformed"}})),
                Some(name)
            );
        }
        for name in ["nyxid__list_schedules", "nyxid__get_agent_skills"] {
            assert_eq!(review_tool(&json!({"name":name,"arguments":{}})), None);
            assert_eq!(
                review_tool(&json!({"name":"nyx__call_tool","arguments":{
                "tool_name":name,"arguments":{}}})),
                None
            );
        }
    }

    #[test]
    fn retry_provider_identity_decoding_preserves_business_semantics() {
        let args = json!({"body":{"value":"中文"},"arguments_json":"{\"provider\":1}",
            "acknowledgement_id":"provider-value","_nyxid_via":"connection-a"});
        let direct = json!({"name":"nyx-vendor__write","arguments":args});
        let encoded = json!({"name":"nyx__call_tool","arguments":{"tool_name":"nyx-vendor__write",
            "arguments_json":args.to_string(),"arguments":{"different":true},"ignored":"decoration"}});
        let nested = json!({"name":"nyx__call_tool","arguments":{"tool_name":"nyx-vendor__write",
            "arguments":args,"arguments_json":42,"body":"ignored"}});
        assert_eq!(digest(&direct), digest(&encoded));
        assert_eq!(digest(&direct), digest(&nested));
        assert_eq!(decode_arguments(&encoded["arguments"]).unwrap(), args);
        assert_eq!(decode_arguments(&nested["arguments"]).unwrap(), args);
        assert_eq!(provider_name(&direct), Some("nyx-vendor__write"));
        assert_eq!(provider_name(&encoded), Some("nyx-vendor__write"));
        assert_eq!(provider_name(&nested), Some("nyx-vendor__write"));
        for (field, value) in [
            ("arguments_json", json!({"provider":1})),
            ("arguments_json", json!("{ \"provider\":1}")),
            ("acknowledgement_id", json!("another-provider-value")),
            ("_nyxid_via", json!("connection-b")),
            ("body", json!({"value":"different"})),
        ] {
            let mut changed = direct.clone();
            changed["arguments"][field] = value;
            assert_ne!(digest(&direct), digest(&changed), "{field}");
        }
        // Old digest parsed this real provider field; v2 must preserve its type.
        let mut typed = direct.clone();
        typed["arguments"]["arguments_json"] = json!({"provider":1});
        assert_eq!(
            super::super::chat_activity_digest(&direct),
            super::super::chat_activity_digest(&typed)
        );
        assert_ne!(digest(&direct), digest(&typed));
        let invalid = json!({"name":"nyx__call_tool","arguments":{"tool_name":"nyx-vendor__write",
            "arguments_json":"{malformed","arguments":args}});
        assert!(decode_arguments(&invalid["arguments"]).is_err());
        // Name extraction borrows just the name, independently of decoding.
        assert_eq!(provider_name(&invalid), Some("nyx-vendor__write"));
        assert_ne!(digest(&invalid), digest(&direct));
        assert_eq!(
            digest(&json!({"name":"nyx-vendor__write"})),
            digest(&json!({"name":"nyx-vendor__write","arguments":{}}))
        );
        for inner in [Value::Null, json!([1, 2]), json!("literal")] {
            assert_eq!(
                decode_arguments(
                    &json!({"tool_name":"x","arguments_json":inner.to_string(),"arguments":{}})
                )
                .unwrap(),
                inner
            );
            assert_eq!(
                decode_arguments(&json!({"tool_name":"x","arguments":inner})).unwrap(),
                inner
            );
        }
        let flat = json!({"tool_name":"nyx-vendor__write","method":"POST","path":"/items","arguments_json":false});
        let expected = json!({"method":"POST","path":"/items"});
        assert_eq!(decode_arguments(&flat).unwrap(), expected);
        let wrapped = json!({"name":"nyx__call_tool","arguments":flat});
        assert_eq!(
            digest(&wrapped),
            digest(&json!({"name":"nyx-vendor__write","arguments":expected}))
        );
        let mut projected = encoded.clone();
        projected["arguments"]["result_query"] = json!("Overview 中文");
        assert_ne!(digest(&projected), digest(&direct));
        let mut changed_projection = projected.clone();
        changed_projection["arguments"]["result_query"] = json!("Approach 中文");
        assert_ne!(digest(&projected), digest(&changed_projection));
        let mut padded_projection = projected.clone();
        padded_projection["arguments"]["result_query"] = json!(" \tOverview 中文\n ");
        assert_eq!(digest(&projected), digest(&padded_projection));
        assert_eq!(
            business_call(&projected).unwrap()["result_query"],
            "Overview 中文"
        );
        for query in [
            Value::Null,
            json!({"section":"unsupported"}),
            json!(7),
            json!("   "),
        ] {
            let mut invalid_projection = projected.clone();
            invalid_projection["arguments"]["result_query"] = query;
            assert!(business_call(&invalid_projection).is_none());
            assert_ne!(digest(&invalid_projection), digest(&projected));
            assert_ne!(digest(&invalid_projection), digest(&direct));
        }
        let mut literal_query = direct.clone();
        literal_query["arguments"]["result_query"] = projected["arguments"]["result_query"].clone();
        assert_ne!(digest(&projected), digest(&literal_query));
        let mut different_tool = direct.clone();
        different_tool["name"] = json!("nyx-vendor__other");
        assert_ne!(digest(&direct), digest(&different_tool));
        // Native normalization remains closed: provider fields never inherit it.
        for (name, field) in [
            ("nyxid__create_schedule", "acknowledgement_id"),
            ("nyxid__remember", "recovery"),
        ] {
            let mut native = json!({"name":name,"arguments":{"value":"same"}});
            let before = digest(&native);
            native["arguments"][field] = json!("link");
            assert_eq!(before, digest(&native));
            let mut provider = native.clone();
            provider["name"] = json!("nyx-vendor__write");
            let before = digest(&provider);
            provider["arguments"][field] = json!("changed");
            assert_ne!(before, digest(&provider));
        }
    }
}
