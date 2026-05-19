const MAX_SCHEMA_BINARY_TRAVERSAL_DEPTH: usize = 64;

pub(crate) fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase()
}

pub(crate) fn is_json_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    normalized == "application/json" || normalized.ends_with("+json")
}

pub(crate) fn is_text_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    normalized.starts_with("text/")
        || is_json_content_type(&normalized)
        || normalized == "application/xml"
        || normalized.ends_with("+xml")
        || normalized == "application/x-www-form-urlencoded"
        || normalized == "application/yaml"
        || normalized == "application/x-yaml"
        || normalized.ends_with("+yaml")
        || normalized == "application/graphql"
        || normalized == "application/javascript"
        || normalized == "application/ecmascript"
        || normalized == "application/sql"
        || normalized == "application/toml"
        || normalized == "application/ndjson"
        || normalized == "application/x-ndjson"
        || normalized == "application/csv"
        || normalized == "application/tsv"
}

pub(crate) fn is_binary_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    normalized == "application/octet-stream"
        || normalized == "application/zip"
        || normalized == "application/gzip"
        || normalized == "application/pdf"
        || normalized.starts_with("image/")
        || normalized.starts_with("audio/")
        || normalized.starts_with("video/")
        || normalized.starts_with("font/")
        || (normalized.starts_with("application/") && !is_text_content_type(&normalized))
}

pub(crate) fn schema_is_binary(schema: Option<&serde_json::Value>) -> bool {
    schema
        .and_then(|schema| schema.get("format"))
        .and_then(|format| format.as_str())
        == Some("binary")
}

pub(crate) fn schema_contains_binary_field(schema: Option<&serde_json::Value>) -> bool {
    schema_contains_binary_field_inner(schema, 0)
}

fn schema_contains_binary_field_inner(schema: Option<&serde_json::Value>, depth: usize) -> bool {
    if depth >= MAX_SCHEMA_BINARY_TRAVERSAL_DEPTH {
        return false;
    }

    let Some(schema) = schema else {
        return false;
    };

    if schema_is_binary(Some(schema)) {
        return true;
    }

    if let Some(properties) = schema.get("properties").and_then(|value| value.as_object())
        && properties.values().any(|property_schema| {
            schema_contains_binary_field_inner(Some(property_schema), depth + 1)
        })
    {
        return true;
    }

    if let Some(items) = schema.get("items")
        && schema_contains_binary_field_inner(Some(items), depth + 1)
    {
        return true;
    }

    if let Some(additional_properties) = schema.get("additionalProperties")
        && schema_contains_binary_field_inner(Some(additional_properties), depth + 1)
    {
        return true;
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(variants) = schema.get(key).and_then(|value| value.as_array())
            && variants.iter().any(|variant_schema| {
                schema_contains_binary_field_inner(Some(variant_schema), depth + 1)
            })
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_contains_binary_field_finds_nested_binary_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "format": "binary"
                }
            }
        });

        assert!(schema_contains_binary_field(Some(&schema)));
    }

    #[test]
    fn schema_contains_binary_field_stops_at_depth_limit() {
        let mut schema = serde_json::json!({
            "type": "string",
            "format": "binary"
        });

        for _ in 0..MAX_SCHEMA_BINARY_TRAVERSAL_DEPTH {
            schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "next": schema
                }
            });
        }

        assert!(!schema_contains_binary_field(Some(&schema)));
    }
}
