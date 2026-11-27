use serde::{Deserialize, Serialize};

/// Compact cosmetic choices. Unknown templates and bounded future choices
/// survive reads; writes are validated against the embedded catalogue.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Avatar {
    #[serde(default, deserialize_with = "read_choices")]
    pub choices: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub template: String,
}

/// Stored data is untrusted. Ignore malformed entries without rejecting the agent.
fn read_choices<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<std::collections::BTreeMap<String, String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let identifier = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && s.as_bytes()[0].is_ascii_lowercase()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    };
    Ok(value
        .as_object()
        .into_iter()
        .flat_map(|v| v.iter())
        .filter_map(|(k, v)| {
            v.as_str()
                .filter(|v| identifier(k) && identifier(v))
                .map(|v| (k.clone(), v.to_owned()))
        })
        .take(32)
        .collect())
}
