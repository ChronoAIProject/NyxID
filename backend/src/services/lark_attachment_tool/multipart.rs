use crate::errors::{AppError, AppResult};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

// Provider schema order, validated as part of the trusted projection. The file
// is always last, independently of JSON map ordering on the caller's machine.
const FIELDS: &[&str] = &["file_name", "parent_type", "parent_node", "size", "extra"];

fn invalid() -> AppError {
    AppError::BadRequest("Invalid attachment upload fields".into())
}
fn scalar(value: &str) -> AppResult<()> {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}
fn filename(value: &str) -> AppResult<()> {
    scalar(value)?;
    if value.trim().is_empty() || matches!(value, "." | "..") || value.contains(['/', '\\']) {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) fn fields(args: &Value, label: &str, size: usize) -> AppResult<Vec<(String, String)>> {
    let object = args.as_object().ok_or_else(invalid)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "attachment_id" | "file_name" | "parent_type" | "parent_node" | "size" | "extra"
        )
    }) {
        return Err(invalid());
    }
    let name = match object.get("file_name") {
        Some(value) => value.as_str().ok_or_else(invalid)?.to_owned(),
        None => super::super::assistant_upload_service::safe_name(label),
    };
    filename(&name)?;
    let kind = args["parent_type"].as_str().ok_or_else(invalid)?;
    if !matches!(kind, "bitable_image" | "bitable_file") {
        return Err(invalid());
    }
    let parent = args["parent_node"].as_str().ok_or_else(invalid)?;
    scalar(parent)?;
    if object
        .get("size")
        .is_some_and(|value| value.as_u64() != Some(size as u64))
    {
        return Err(AppError::BadRequest(
            "Attachment size does not match stored bytes".into(),
        ));
    }
    let extra = json!({"drive_route_token":parent});
    if let Some(value) = object.get("extra") {
        let supplied: Value =
            serde_json::from_str(value.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
        if supplied != extra {
            return Err(invalid());
        }
    }
    let result = vec![
        ("file_name".into(), name),
        ("parent_type".into(), kind.into()),
        ("parent_node".into(), parent.into()),
        ("size".into(), size.to_string()),
        ("extra".into(), extra.to_string()),
    ];
    for (_, value) in &result {
        scalar(value)?;
    }
    Ok(result)
}

/// RFC 7578 quoted-string disposition parameters; controls were refused before
/// quoting, and only server-owned field names reach the encoder.
fn quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn encode(
    fields: &[(String, String)],
    content_type: &str,
    file: &[u8],
) -> AppResult<(bytes::Bytes, String)> {
    let mut names = HashSet::new();
    if fields.len() != FIELDS.len() {
        return Err(invalid());
    }
    for ((name, value), expected) in fields.iter().zip(FIELDS) {
        if name != expected || !names.insert(name) {
            return Err(invalid());
        }
        scalar(value)?;
    }
    filename(&fields[0].1)?;
    if content_type.len() > 128
        || content_type.chars().any(char::is_control)
        || content_type.parse::<mime::Mime>().is_err()
    {
        return Err(invalid());
    }
    let mut digest = Sha256::new();
    digest.update(file);
    digest.update(serde_json::to_vec(fields).map_err(|_| invalid())?);
    let boundary = format!("nyxid-{}", &hex::encode(digest.finalize())[..32]);
    let mut body = Vec::with_capacity(file.len() + 8192);
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{value}\r\n",
                quote(name)
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n",quote(&fields[0].1)).as_bytes());
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok((
        body.into(),
        format!("multipart/form-data; boundary={boundary}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args() -> Value {
        json!({"attachment_id":"id","parent_type":"bitable_image","parent_node":"base"})
    }

    #[test]
    fn lark_attachment_multipart_golden_body_is_stable_and_file_is_last() {
        let fields = fields(&args(), "receipt.png", 3).unwrap();
        let (body, content_type) = encode(&fields, "image/png", b"PNG").unwrap();
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        assert_eq!(boundary, "nyxid-bc0bacfe4db9b12dac9378b69b5ff264");
        let expected = concat!(
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"file_name\"\r\n\r\nreceipt.png\r\n",
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"parent_type\"\r\n\r\nbitable_image\r\n",
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"parent_node\"\r\n\r\nbase\r\n",
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"size\"\r\n\r\n3\r\n",
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"extra\"\r\n\r\n{\"drive_route_token\":\"base\"}\r\n",
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"receipt.png\"\r\nContent-Type: image/png\r\n\r\nPNG\r\n--BOUNDARY--\r\n"
        ).replace("BOUNDARY",boundary);
        assert_eq!(body.as_ref(), expected.as_bytes());
        assert_eq!(
            encode(&fields, "image/png", b"PNG").unwrap(),
            (body.clone(), content_type.clone())
        );
        assert_ne!(
            encode(&fields, "image/png", b"NEW").unwrap().1,
            content_type
        );
        assert_eq!(quote("a\"b"), "a\\\"b");
    }

    #[test]
    fn lark_attachment_multipart_refuses_names_controls_duplicates_reserved_oversized_and_size() {
        for name in [
            "", ".", "..", "../x", "x/y", "x\\y", "x\rname", "x\nname", "x\0name",
        ] {
            let mut value = args();
            value["file_name"] = json!(name);
            assert!(fields(&value, "label", 3).is_err(), "{name:?}");
        }
        for (key, value) in [
            ("file_name", json!("x".repeat(1025))),
            ("parent_node", json!("x".repeat(1025))),
            ("parent_node", json!("base\r\nInjected: yes")),
            ("file", json!("raw")),
            ("_method", json!("DELETE")),
            ("filename", json!("reserved")),
            ("size", json!(2)),
            ("size", json!("3")),
            ("parent_type", json!("other")),
            ("extra", json!("{\"drive_route_token\":\"other\"}")),
        ] {
            let mut args = args();
            args[key] = value;
            assert!(fields(&args, "label", 3).is_err(), "{key}");
        }
        let mut fields = fields(&args(), "label", 3).unwrap();
        fields[1].0 = fields[0].0.clone();
        assert!(encode(&fields, "image/png", b"PNG").is_err());
        fields[1].0 = "file".into();
        assert!(encode(&fields, "image/png", b"PNG").is_err());
        assert!(encode(&fields, "image/png\r\nBad: header", b"PNG").is_err());
    }
}
