//! Operation-scoped Sandbox adapter. Never interprets a URL or upstream MIME.
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const RESPONSE_BYTES: usize = 13 * 1024 * 1024;
pub const TOTAL_BYTES: usize = 9 * 1024 * 1024;
pub const FILE_BYTES: usize = 8 * 1024 * 1024;
pub const IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const RESERVATION: usize = RESPONSE_BYTES + TOTAL_BYTES;
pub const MODE: &str = "generated_artifact_v1";
pub type Checked<T> = Result<T, &'static str>;

#[derive(Deserialize)]
struct ExecuteResponse {
    success: bool,
    output: Option<Output>,
}
#[derive(Deserialize)]
struct Output {
    #[serde(rename = "stdout", deserialize_with = "discard_text")]
    _stdout: (),
    #[serde(rename = "stderr", deserialize_with = "discard_text")]
    _stderr: (),
    exit_code: i64,
    #[serde(default, deserialize_with = "bounded_files")]
    files: Option<Vec<RetrievedFile>>,
}
// These streams are arbitrary file content, paths or secrets. Validate their
// wire type without retaining them in the generated-artifact result.
fn discard_text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    struct Text;
    impl<'de> serde::de::Visitor<'de> for Text {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a string")
        }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
            Ok(())
        }
    }
    deserializer.deserialize_str(Text)
}

/// Only primitive, selected metadata may cross the generated-result context boundary.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Summary {
    success: bool,
    exit_code: Option<i64>,
    verified_file_count: usize,
}
/// Reconstruct rather than forward saved text: older summaries can contain
/// process output even when the watch has the generated-artifact marker.
pub fn context_summary(text: &str) -> Zeroizing<String> {
    serde_json::from_str::<Summary>(text)
        .ok()
        .filter(|summary| summary.verified_file_count <= 8)
        .and_then(|summary| serde_json::to_string(&summary).ok())
        .map(Zeroizing::new)
        .unwrap_or_else(|| Zeroizing::new(String::new()))
}
fn bounded_files<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<RetrievedFile>>, D::Error> {
    struct Files;
    impl<'de> serde::de::Visitor<'de> for Files {
        type Value = Option<Vec<RetrievedFile>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most eight result files or null")
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut files = Vec::with_capacity(8);
            while let Some(file) = seq.next_element::<RetrievedFile>()? {
                if files.len() == 8 {
                    return Err(serde::de::Error::custom("artifact_too_large"));
                }
                files.push(file);
            }
            Ok(Some(files))
        }
    }
    deserializer.deserialize_any(Files)
}
#[derive(Deserialize)]
struct RetrievedFile {
    path: String,
    content: Option<Zeroizing<String>>,
    error: Option<String>,
    size: Option<u64>,
}
pub struct File {
    pub filename: String,
    pub source_path: String,
    pub content_type: &'static str,
    pub preview: &'static str,
    pub bytes: Zeroizing<Vec<u8>>,
    pub sha256: String,
}
pub struct ResultFiles {
    pub files: Vec<File>,
    pub identity: String,
    pub summary: Zeroizing<String>,
    pub success: bool,
}
pub struct Status {
    pub expires_at: Option<DateTime<Utc>>,
    pub version: Option<String>,
}
pub fn status(value: &serde_json::Value, operation: &str, now: DateTime<Utc>) -> Checked<Status> {
    if value["operation_id"].as_str() != Some(operation) {
        return Err("artifact_source_changed");
    }
    let expires_at = match value.get("expires_at").filter(|v| !v.is_null()) {
        Some(v) => Some(
            DateTime::parse_from_rfc3339(v.as_str().ok_or("artifact_invalid")?)
                .map_err(|_| "artifact_invalid")?
                .with_timezone(&Utc),
        ),
        None => None,
    };
    if expires_at.is_some_and(|expiry| expiry <= now) {
        return Err("result_expired");
    }
    if value["status"] != "succeeded" || value["result_available"] != true {
        return Err("result_unavailable");
    }
    Ok(Status {
        expires_at,
        version: match value.get("version").filter(|v| !v.is_null()) {
            Some(version) => Some(version.as_u64().ok_or("artifact_invalid")?.to_string()),
            None => None,
        },
    })
}
pub fn filename(path: &str, index: usize) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut safe = String::new();
    for c in base.chars().filter(|c| {
        !c.is_control()
            && !matches!(c, '/' | '\\')
            && !matches!(*c as u32, 0x202a..=0x202e | 0x2066..=0x2069)
    }) {
        if safe.len() + c.len_utf8() > 160 {
            break;
        }
        safe.push(c);
    }
    let safe = safe.trim().trim_matches('.');
    if safe.is_empty() {
        format!("generated-file-{}", index + 1)
    } else {
        safe.into()
    }
}
fn part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
pub fn parse(body: &str, operation: &str) -> Checked<ResultFiles> {
    if body.len() > RESPONSE_BYTES {
        return Err("artifact_too_large");
    }
    let result: ExecuteResponse = serde_json::from_str(body).map_err(|error| {
        if error.to_string().starts_with("artifact_too_large") {
            "artifact_too_large"
        } else {
            "artifact_invalid"
        }
    })?;
    let mut files = Vec::new();
    let mut total = 0usize;
    let mut hash = Sha256::new();
    part(&mut hash, b"nyxid-sandbox-result-v1");
    part(&mut hash, operation.as_bytes());
    let exit_code = result.output.as_ref().map(|output| output.exit_code);
    if let Some(output) = result.output {
        let entries = output.files.unwrap_or_default();
        if entries.len() > 8 {
            return Err("artifact_too_large");
        }
        for (index, entry) in entries.into_iter().enumerate() {
            if entry.error.is_some() {
                return Err("artifact_invalid");
            }
            let encoded = entry.content.ok_or("artifact_invalid")?;
            let remaining = FILE_BYTES.min(TOTAL_BYTES.saturating_sub(total));
            let encoded_max = remaining
                .div_ceil(3)
                .checked_mul(4)
                .ok_or("artifact_too_large")?;
            if encoded.len() > encoded_max {
                return Err("artifact_too_large");
            }
            let bytes = Zeroizing::new(
                STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|_| "artifact_invalid")?,
            );
            if bytes.is_empty() {
                return Err("artifact_invalid");
            }
            if bytes.len() > remaining {
                return Err("artifact_too_large");
            }
            total = total.checked_add(bytes.len()).ok_or("artifact_too_large")?;
            let name = filename(&entry.path, index);
            let (content_type, preview) = verified_type(&name, &bytes)?;
            // Upstream size is diagnostic only; decoded size is authoritative.
            let _declared_size_mismatch = entry.size.is_some_and(|size| size != bytes.len() as u64);
            part(&mut hash, name.as_bytes());
            part(&mut hash, content_type.as_bytes());
            part(&mut hash, &bytes);
            let sha256 = hex::encode(Sha256::digest(&*bytes));
            files.push(File {
                filename: name,
                source_path: bounded(&entry.path, 512).into(),
                content_type,
                preview,
                bytes,
                sha256,
            });
        }
    }
    let summary = Zeroizing::new(
        serde_json::to_string(&Summary {
            success: result.success,
            exit_code,
            verified_file_count: files.len(),
        })
        .map_err(|_| "artifact_invalid")?,
    );
    Ok(ResultFiles {
        files,
        identity: hex::encode(hash.finalize()),
        summary,
        success: result.success,
    })
}
fn bounded(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn verified_type(name: &str, bytes: &[u8]) -> Checked<(&'static str, &'static str)> {
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let candidate = match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    };
    if let Some(mime) = candidate {
        if bytes.len() > IMAGE_BYTES {
            return Err("artifact_too_large");
        }
        if !super::super::mcp_service::image_magic_matches(mime, bytes) {
            return Err("artifact_invalid");
        }
        return Ok((mime, "image"));
    }
    if ext == "mp4" {
        return Ok((
            "video/mp4",
            if super::mp4::validate(bytes)? {
                "video"
            } else {
                "download"
            },
        ));
    }
    Ok(("application/octet-stream", "download"))
}
