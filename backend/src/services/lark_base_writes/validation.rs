use super::*;
use std::collections::{BTreeMap, HashSet};

pub(super) fn body(value: &Value) -> AppResult<(Value, BTreeMap<String, Vec<String>>)> {
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != 2 || !object.contains_key("fields") || !object.contains_key("attachments") {
        return Err(invalid());
    }
    let fields = value["fields"].as_object().ok_or_else(invalid)?;
    let attachments = value["attachments"]
        .as_object()
        .filter(|map| !map.is_empty())
        .ok_or_else(invalid)?;
    let mut seen = HashSet::new();
    let mut assignments = BTreeMap::new();
    for (field, ids) in attachments {
        if field.is_empty() || fields.contains_key(field) {
            return Err(invalid());
        }
        let ids = ids
            .as_array()
            .filter(|ids| !ids.is_empty())
            .ok_or_else(invalid)?;
        let mut list = Vec::new();
        for id in ids {
            let id = id
                .as_str()
                .filter(|id| Uuid::parse_str(id).is_ok())
                .ok_or_else(invalid)?;
            if !seen.insert(id.to_owned()) || seen.len() > 10 {
                return Err(invalid());
            }
            list.push(id.to_owned());
        }
        assignments.insert(field.clone(), list);
    }
    Ok((
        mcp::canonical_json(Value::Object(fields.clone())),
        assignments,
    ))
}

pub(super) fn token(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn validate_fields(
    fields: &Value,
    attachments: &BTreeMap<String, Vec<String>>,
    schema: &BTreeMap<String, Value>,
) -> AppResult<()> {
    for field in attachments.keys() {
        if schema.get(field).and_then(|s| s["type"].as_i64()) != Some(17) {
            return Err(AppError::BadRequest(
                "Unknown or non-attachment Base field".into(),
            ));
        }
    }
    for (field, value) in fields.as_object().ok_or_else(invalid)? {
        let spec = schema
            .get(field)
            .ok_or_else(|| AppError::BadRequest("Unknown Base field".into()))?;
        match spec["type"].as_i64() {
            Some(17) => {
                return Err(AppError::BadRequest(
                    "Attachment fields must use attachments".into(),
                ));
            }
            Some(2) => {
                if !value.as_f64().is_some_and(f64::is_finite) {
                    return Err(AppError::BadRequest(
                        "Base number field requires a finite number".into(),
                    ));
                }
            }
            Some(kind @ (3 | 4)) => {
                let values = if kind == 3 {
                    vec![value]
                } else {
                    value.as_array().ok_or_else(invalid)?.iter().collect()
                };
                let options = spec["property"]["options"].as_array().ok_or_else(invalid)?;
                for value in values {
                    let name = value.as_str().ok_or_else(invalid)?;
                    if !options
                        .iter()
                        .any(|option| option["name"].as_str() == Some(name))
                    {
                        return Err(AppError::BadRequest(
                            "Base select option does not exist; options will not be created".into(),
                        ));
                    }
                }
            }
            Some(_) => {}
            None => return Err(invalid()),
        }
    }
    Ok(())
}

pub(super) fn logical_digest(
    fields: &Value,
    assignments: &BTreeMap<String, Vec<String>>,
    uploads: &[InputUpload],
) -> AppResult<String> {
    let mut content = BTreeMap::new();
    for (field, ids) in assignments {
        let hashes: AppResult<Vec<_>> = ids
            .iter()
            .map(|id| {
                uploads
                    .iter()
                    .find(|upload| upload.id == *id)
                    .map(|upload| upload.sha256.clone())
                    .ok_or_else(invalid)
            })
            .collect();
        content.insert(field, hashes?);
    }
    Ok(mcp::canonical_sha256(
        json!({"fields":fields,"attachments":content}),
    ))
}
