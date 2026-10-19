//! Path parameter encoding shared by MCP, node dispatch and REST exact approvals
//! through `mcp_service::build_proxy_args`. This never relaxes proxy validation.

use crate::errors::{AppError, AppResult};

pub(crate) const PATH_SEGMENTS: &str = "x-nyxid-path-segments";

pub(crate) fn encode_parameter(parameter: &serde_json::Value, value: &str) -> AppResult<String> {
    let multi_segment = parameter["in"] == "path"
        && (parameter["allowReserved"].as_bool() == Some(true)
            || parameter[PATH_SEGMENTS].as_bool() == Some(true));
    if !multi_segment {
        return Ok(urlencoding::encode(value).into_owned());
    }

    // An entirely empty value denotes zero segments (GitHub's repository root).
    // Empty segments *within* a nonempty value, including either end, are invalid.
    if value.is_empty() {
        return Ok(String::new());
    }
    let invalid = || AppError::BadRequest("Invalid multi-segment path parameter".into());
    let mut encoded = Vec::new();
    for segment in value.split('/') {
        let mut decoded = std::borrow::Cow::Borrowed(segment);
        // Refuse nested encodings even if a downstream router decodes repeatedly.
        // Overly deep input is rejected, never accepted after a truncated check.
        let mut stable = false;
        for _ in 0..8 {
            if matches!(decoded.as_ref(), "" | "." | "..")
                || decoded.contains(['/', '\\'])
                || decoded.chars().any(char::is_control)
            {
                return Err(invalid());
            }
            let next = urlencoding::decode(&decoded).map_err(|_| invalid())?;
            if next == decoded {
                stable = true;
                break;
            }
            decoded = std::borrow::Cow::Owned(next.into_owned());
        }
        if !stable {
            return Err(invalid());
        }
        encoded.push(urlencoding::encode(segment));
    }
    let encoded = encoded.join("/");
    super::proxy_service::validate_requested_proxy_path(&encoded)?;
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn proxy_path_segments_opt_in_and_default_encoding() {
        for marker in ["allowReserved", PATH_SEGMENTS] {
            let mut parameter = json!({"name":"anything", "in":"path"});
            parameter[marker] = json!(true);
            for (input, expected) in [
                ("", ""),
                ("backend", "backend"),
                ("backend/src", "backend/src"),
                ("backend/src/a b.rs", "backend/src/a%20b.rs"),
                ("日本語/file.txt", "%E6%97%A5%E6%9C%AC%E8%AA%9E/file.txt"),
            ] {
                let path = encode_parameter(&parameter, input).unwrap();
                assert_eq!(path, expected);
                super::super::proxy_service::validate_requested_proxy_path(&path).unwrap();
            }
            parameter[marker] = json!(false);
            assert_eq!(encode_parameter(&parameter, "a/b").unwrap(), "a%2Fb");
        }
        let named_path = json!({"name":"path", "in":"path"});
        assert_eq!(encode_parameter(&named_path, "a/b").unwrap(), "a%2Fb");
    }

    #[test]
    fn proxy_path_segments_reject_traversal_and_nested_encodings() {
        let parameter = json!({"in":"path", "x-nyxid-path-segments":true});
        for value in [
            "../x",
            "a/../b",
            "a//b",
            "a/%2e%2e/b",
            "a/%2Fb",
            "a\\b",
            ".",
            "a/./b",
            "/a",
            "a/",
            "a/%5cb",
            "a/%252fb",
            "a/.%2E/b",
            "a/%252e%252e/b",
            "a/\n/b",
            "a/%0A/b",
            "a/%2500/b",
            "a/\u{7f}",
            "a/\u{85}",
            "a/%C2%85",
            "a?query",
            "a#fragment",
        ] {
            assert!(encode_parameter(&parameter, value).is_err(), "{value:?}");
        }
    }
}
