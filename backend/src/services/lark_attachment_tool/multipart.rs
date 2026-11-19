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

/// The caller's argument object is immutable from scope admission through field encoding.
/// Only this trusted materializer can attach its JSON to a prepared multipart request.
pub(crate) struct UploadArguments<'a> {
    pub(super) args: &'a Value,
    pub(super) scope_json: Option<Vec<u8>>,
}

impl<'a> UploadArguments<'a> {
    pub(crate) fn new(args: &'a Value, body_limited: bool) -> AppResult<Self> {
        let scope_json = if body_limited {
            Some(serde_json::to_vec(args).map_err(|_| invalid())?)
        } else {
            None
        };
        Ok(Self { args, scope_json })
    }

    pub(crate) fn scope_json(&self) -> Option<&[u8]> {
        self.scope_json.as_deref()
    }

    pub(super) fn fields(&self, label: &str, size: usize) -> AppResult<Vec<(String, String)>> {
        build_fields(self.args, label, size, self.scope_json.is_some())
    }
}

pub(crate) fn fields(args: &Value, label: &str, size: usize) -> AppResult<Vec<(String, String)>> {
    build_fields(args, label, size, false)
}

fn build_fields(
    args: &Value,
    label: &str,
    size: usize,
    preserve_extra: bool,
) -> AppResult<Vec<(String, String)>> {
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
    let mut wire_extra = extra.to_string();
    if let Some(value) = object.get("extra") {
        let supplied = value.as_str().ok_or_else(invalid)?;
        let parsed: Value = if preserve_extra {
            // The verbatim string will reach the provider. Reject ambiguous duplicate keys.
            nyxid_permissions::parse_json(supplied.as_bytes()).map_err(|_| invalid())?
        } else {
            serde_json::from_str(supplied).map_err(|_| invalid())?
        };
        if parsed != extra {
            return Err(invalid());
        }
        if preserve_extra {
            wire_extra = supplied.to_owned();
        }
    }
    let result = vec![
        ("file_name".into(), name),
        ("parent_type".into(), kind.into()),
        ("parent_node".into(), parent.into()),
        ("size".into(), size.to_string()),
        ("extra".into(), wire_extra),
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
    encode_parts(fields, &fields[0].1, "file", content_type, file, &boundary)
}

pub(crate) fn encode_image(
    name: &str,
    content_type: &str,
    file: &[u8],
) -> AppResult<(bytes::Bytes, String)> {
    filename(name)?;
    let boundary = format!("nyxid-{}", uuid::Uuid::new_v4().simple());
    encode_parts(
        &[("image_type".into(), "message".into())],
        name,
        "image",
        content_type,
        file,
        &boundary,
    )
}

fn encode_parts(
    fields: &[(String, String)],
    filename: &str,
    file_field: &str,
    content_type: &str,
    file: &[u8],
    boundary: &str,
) -> AppResult<(bytes::Bytes, String)> {
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
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n",quote(filename)).as_bytes());
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
    fn lark_attachment_supplied_extra_matches_scoped_wire_bytes() {
        let mut input = args();
        let formatted = r#"{ "drive_route_token" : "base" }"#;
        input["extra"] = json!(formatted);
        let upload = UploadArguments::new(&input, true).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(upload.scope_json.as_deref().unwrap()).unwrap(),
            input
        );
        let fields = upload.fields("receipt.png", 3).unwrap();
        assert_eq!(fields[4].1, formatted);
        let (body, _) = encode(&fields, "image/png", b"PNG").unwrap();
        assert!(
            std::str::from_utf8(&body)
                .unwrap()
                .contains(&format!("name=\"extra\"\r\n\r\n{formatted}\r\n"))
        );
        let ordinary = UploadArguments::new(&input, false).unwrap();
        assert_eq!(
            ordinary.fields("receipt.png", 3).unwrap()[4].1,
            r#"{"drive_route_token":"base"}"#
        );
        for ambiguous in [
            r#"{"drive_route_token":"other","drive_route_token":"base"}"#,
            r#"{"drive_route_token":"other"}"#,
        ] {
            input["extra"] = json!(ambiguous);
            let upload = UploadArguments::new(&input, true).unwrap();
            assert!(upload.fields("receipt.png", 3).is_err());
        }
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
