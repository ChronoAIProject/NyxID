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

pub(super) fn provider_name(params: &Value) -> Option<&str> {
    let name = params["name"].as_str()?;
    let name = if name == "nyx__call_tool" {
        params["arguments"]["tool_name"]
            .as_str()
            .filter(|name| !name.is_empty() && name.len() <= 200)?
    } else {
        name
    };
    (!super::native_activity_names().contains(name)).then_some(name)
}

/// Old persisted activities retain their exact digests. Match the supplied form
/// and standard historical direct/JSON/nested/flat encodings in either direction.
/// Ignored arbitrary wrapper extras cannot be reconstructed from an old hash.
pub(super) fn matching_digests(params: &Value) -> Vec<String> {
    let mut values = vec![
        digest(params),
        super::chat_activity_digest(params),
        super::legacy_chat_retry_digest(params),
    ];
    if let Some(call) = business_call(params) {
        let name = &call["name"];
        let args = &call["arguments"];
        if call.get("result_query").is_none() {
            let direct = json!({"name":name,"arguments":args});
            values.push(super::chat_activity_digest(&direct));
            values.push(super::legacy_chat_retry_digest(&direct));
            if args == &json!({}) {
                values.push(super::chat_activity_digest(&json!({"name":name})));
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
            let wrapped = json!({"name":"nyx__call_tool","arguments":wrapper});
            values.push(super::chat_activity_digest(&wrapped));
            values.push(super::legacy_chat_retry_digest(&wrapped));
        }
    }
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

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
