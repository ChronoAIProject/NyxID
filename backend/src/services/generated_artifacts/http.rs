//! HTTP policy is separate from file validation and the private owner read service.
use super::*;
use crate::errors::{AppError, AppResult};
use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::Response,
};

#[derive(Debug, PartialEq, Eq)]
pub enum RangeSelection {
    Ignore,
    Unsatisfiable,
    Partial(usize, usize),
}
/// Unknown units, malformed syntax and unsupported multi-ranges are ignored.
/// Only a syntactically valid single byte range can produce 416.
pub fn select_range(header: &str, size: usize) -> RangeSelection {
    use RangeSelection::*;
    let Some(range) = header.strip_prefix("bytes=") else {
        return Ignore;
    };
    if range.contains(',') {
        return Ignore;
    }
    let Some((start_text, end_text)) = range.trim().split_once('-') else {
        return Ignore;
    };
    let number = |s: &str| {
        if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
            None
        } else {
            // RFC 9110 permits arbitrarily long decimal positions. Saturation
            // preserves comparisons with the bounded file size without overflow.
            Some(s.bytes().fold(0usize, |value, digit| {
                value
                    .saturating_mul(10)
                    .saturating_add(usize::from(digit - b'0'))
            }))
        }
    };
    if start_text.is_empty() {
        let Some(suffix) = number(end_text) else {
            return Ignore;
        };
        if suffix == 0 || size == 0 {
            return Unsatisfiable;
        }
        return Partial(size.saturating_sub(suffix), size - 1);
    }
    let Some(start) = number(start_text) else {
        return Ignore;
    };
    let end = if end_text.is_empty() {
        None
    } else {
        let Some(end) = number(end_text) else {
            return Ignore;
        };
        // Compare the full decimal values before saturation so two enormous
        // endpoints in reverse order still count as malformed.
        let first = start_text.trim_start_matches('0');
        let last = end_text.trim_start_matches('0');
        if (last.len(), last) < (first.len(), first) {
            return Ignore;
        }
        Some(end)
    };
    if start >= size {
        return Unsatisfiable;
    }
    Partial(start, end.unwrap_or(size - 1).min(size - 1))
}
fn disposition(filename: Option<&str>, inline: bool) -> String {
    let kind = if inline { "inline" } else { "attachment" };
    match filename {
        None => kind.into(),
        Some(name) => {
            let name = sandbox::filename(name, 0);
            let ascii: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || ".-_ ".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            format!(
                "{kind}; filename=\"{ascii}\"; filename*=UTF-8''{}",
                urlencoding::encode(&name)
            )
        }
    }
}
struct BudgetedBytes {
    bytes: zeroize::Zeroizing<Vec<u8>>,
    _reservation: byte_budget::ByteReservation,
}
impl AsRef<[u8]> for BudgetedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
pub async fn owner_response(
    db: &Database,
    keys: &EncryptionKeys,
    user: &str,
    conversation: &str,
    id: &str,
    request: &HeaderMap,
) -> AppResult<Response> {
    super::super::assistant_nyxagent::get(db, user, conversation).await?;
    let filter = doc! {"_id":id,"user_id":user,"conversation_id":conversation};
    super::super::assistant_upload_retention::require_available(db, filter.clone()).await?;
    let metadata = db
        .collection::<Document>(ATTACHMENTS)
        .find_one(filter)
        .projection(doc! {"size":1,"filename":1,"preview_kind":1,"content_type":1,"origin":1})
        .await?
        .ok_or_else(missing)?;
    let size = metadata
        .get_i64("size")
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0 && *n <= super::super::attachment_extraction::MAX_BYTES)
        .ok_or_else(invalid)?;
    let reservation = byte_budget::reserve(size).await?;
    let (content_type, bytes) =
        super::super::assistant_nyxagent::read_attachment(db, keys, user, conversation, id).await?;
    if bytes.len() != size {
        return Err(AppError::Internal("Attachment size mismatch".into()));
    }
    let generated = metadata.get_str("origin").ok() == Some("generated_artifact");
    let inline = !generated
        || matches!(
            content_type.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        )
        || (content_type == "video/mp4" && metadata.get_str("preview_kind").ok() == Some("video"));
    let mut status = StatusCode::OK;
    let mut content_range = None;
    // Keep the permit with the actual buffer, including clones/slices queued by
    // HTTP after Body has yielded a frame. Stream lifetime alone is too short.
    let mut payload = bytes::Bytes::from_owner(BudgetedBytes {
        bytes: zeroize::Zeroizing::new(bytes),
        _reservation: reservation,
    });
    if !request.contains_key("if-range") && request.contains_key("range") {
        let headers: Vec<_> = request.get_all("range").iter().collect();
        let selected = if headers.len() == 1 {
            headers[0]
                .to_str()
                .ok()
                .map(|h| select_range(h, size))
                .unwrap_or(RangeSelection::Ignore)
        } else {
            RangeSelection::Ignore
        };
        match selected {
            RangeSelection::Partial(start, end) => {
                status = StatusCode::PARTIAL_CONTENT;
                content_range = Some(format!("bytes {start}-{end}/{size}"));
                payload = payload.slice(start..=end);
            }
            RangeSelection::Unsatisfiable => {
                status = StatusCode::RANGE_NOT_SATISFIABLE;
                content_range = Some(format!("bytes */{size}"));
                payload = bytes::Bytes::new();
            }
            RangeSelection::Ignore => {}
        }
    }
    let length = payload.len();
    let mut response = Response::builder()
        .status(status)
        .header(
            "content-type",
            if inline {
                content_type.as_str()
            } else {
                "application/octet-stream"
            },
        )
        .header("content-length", length.to_string())
        .header("accept-ranges", "bytes")
        .header("cache-control", "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header("content-security-policy", "default-src 'none'; sandbox")
        .header(
            "content-disposition",
            disposition(
                if generated {
                    metadata.get_str("filename").ok()
                } else {
                    None
                },
                inline,
            ),
        );
    if let Some(range) = content_range {
        response = response.header("content-range", range);
    }
    response
        .body(Body::from(payload))
        .map_err(|_| AppError::Internal("Invalid attachment response".into()))
}
