//! Bounded Lark filename hints; no hint establishes the file's content type.
use crate::services::{assistant_upload_service::MAX_FILENAME_CHARS, attachment_filename};

pub(super) fn selector(value: &serde_json::Value) -> Option<String> {
    ["file_name", "name", "filename"]
        .into_iter()
        .find_map(|key| {
            attachment_filename::original_name(
                value.get(key).and_then(serde_json::Value::as_str),
                MAX_FILENAME_CHARS,
            )
        })
}

pub(super) fn from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let mut values = headers.get_all(reqwest::header::CONTENT_DISPOSITION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    disposition(std::str::from_utf8(value.as_bytes()).ok()?)
}

fn token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// RFC 6266/8187: prefer a usable UTF-8 filename* over the ASCII filename.
/// Refuse ambiguous, oversized or malformed headers and never retain paths.
pub(super) fn disposition(header: &str) -> Option<String> {
    if header.len() > 4096 || header.chars().any(|c| c.is_control() && c != '\t') {
        return None;
    }
    let (kind, mut rest) = header.split_once(';')?;
    if !matches!(
        kind.trim_matches([' ', '\t']).to_ascii_lowercase().as_str(),
        "attachment" | "inline"
    ) {
        return None;
    }
    let (mut plain, mut extended) = (None, None);
    let (mut saw_plain, mut saw_extended) = (false, false);
    let mut count = 0;
    while !rest.trim_matches([' ', '\t']).is_empty() {
        count += 1;
        if count > 16 {
            return None;
        }
        let (key, tail) = rest.trim_start_matches([' ', '\t']).split_once('=')?;
        let key = key.trim_matches([' ', '\t']);
        if key.is_empty() || !key.bytes().all(token) {
            return None;
        }
        let tail = tail.trim_start_matches([' ', '\t']);
        let quoted = tail.starts_with('"');
        let value;
        if quoted {
            let mut decoded = String::new();
            let mut escaped = false;
            let mut end = None;
            for (index, ch) in tail[1..].char_indices() {
                if escaped {
                    decoded.push(ch);
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    end = Some(index + 2);
                    break;
                } else {
                    decoded.push(ch);
                }
            }
            let remaining = tail[end?..].trim_start_matches([' ', '\t']);
            rest = if remaining.is_empty() {
                ""
            } else {
                remaining.strip_prefix(';')?
            };
            value = decoded;
        } else {
            let (raw, remaining) = tail.split_once(';').unwrap_or((tail, ""));
            let raw = raw.trim_matches([' ', '\t']);
            if raw.is_empty() || !raw.bytes().all(token) {
                return None;
            }
            value = raw.into();
            rest = remaining;
        }
        if key.eq_ignore_ascii_case("filename") {
            if saw_plain {
                return None;
            }
            saw_plain = true;
            plain = attachment_filename::original_name(Some(&value), MAX_FILENAME_CHARS);
        } else if key.eq_ignore_ascii_case("filename*") {
            if saw_extended {
                return None;
            }
            saw_extended = true;
            if !quoted {
                extended = decode_extended(&value);
            }
        }
    }
    extended.or(plain)
}

fn decode_extended(value: &str) -> Option<String> {
    let mut pieces = value.splitn(3, '\'');
    let charset = pieces.next()?;
    if !charset.eq_ignore_ascii_case("utf-8") && !charset.eq_ignore_ascii_case("us-ascii") {
        return None;
    }
    let language = pieces.next()?;
    if !language
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }
    let encoded = pieces.next()?;
    let mut bytes = encoded.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            if !bytes.next()?.is_ascii_hexdigit() || !bytes.next()?.is_ascii_hexdigit() {
                return None;
            }
        } else if !byte.is_ascii_alphanumeric() && !b"!#$&+-.^_`|~".contains(&byte) {
            return None;
        }
    }
    let decoded = urlencoding::decode(encoded).ok()?;
    if charset.eq_ignore_ascii_case("us-ascii") && !decoded.is_ascii() {
        return None;
    }
    attachment_filename::original_name(Some(&decoded), MAX_FILENAME_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_disposition_prefers_unicode_and_refuses_malformed_or_oversized_hints() {
        for (header, expected) in [
            (
                "attachment;\tfilename\t=\t\"Receipt (final).pdf\"\t;\tx!#$%&'*+-.^_`|~=valid-token",
                Some("Receipt (final).pdf"),
            ),
            ("attachment; filename=one.pdf,two", None),
            ("attachment; filename=\"bad\tname.pdf\"", None),
            ("attachment; filename=\"收据.pdf\"", Some("收据.pdf")),
            ("attachment; filename=receipt.pdf", Some("receipt.pdf")),
            (
                "inline; filename=\"original receipt\"",
                Some("original receipt"),
            ),
            (
                "attachment; filename=ascii.pdf; filename*=UTF-8''%E6%94%B6%E6%8D%AE.pdf",
                Some("收据.pdf"),
            ),
            (
                "attachment; filename*=utf-8'en'%E7%85%A7%E7%89%87.jpg",
                Some("照片.jpg"),
            ),
            (
                "attachment; filename=ok.pdf; filename*=UTF-8''bad%ZZ",
                Some("ok.pdf"),
            ),
            ("attachment; filename*=UTF-8''bad%FF", None),
            ("attachment; filename*=UTF-8''bad%0Aname.pdf", None),
            ("attachment; filename=\"unterminated", None),
            ("attachment; filename=one.pdf; filename=two.pdf", None),
            ("attachment; filename=\"x.pdf\"garbage", None),
            ("attachment; filename=..", None),
            ("attachment; filename=bad\r\nname", None),
        ] {
            assert_eq!(disposition(header).as_deref(), expected, "{header}");
        }
        assert!(disposition(&format!("attachment; filename={}", "a".repeat(1025))).is_none());
        assert!(disposition(&"a".repeat(4097)).is_none());
    }

    #[test]
    fn repeated_content_disposition_fields_are_ambiguous() {
        use reqwest::header::{CONTENT_DISPOSITION, HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.append(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment;\tfilename=one.pdf"),
        );
        assert_eq!(from_headers(&headers).as_deref(), Some("one.pdf"));
        headers.append(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=two.pdf"),
        );
        assert!(from_headers(&headers).is_none());
    }
}
