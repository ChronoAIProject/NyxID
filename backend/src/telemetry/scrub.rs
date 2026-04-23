//! Egress scrubber.
//!
//! Every string-valued property passed to the vendor is run through
//! [`scrub_string`] (or [`scrub_value`] recursively for nested JSON) so
//! that no matter how a handler constructs props, the §6 redaction rules
//! are enforced at the single point of egress.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

// --- Regex patterns applied to every string value at egress ------------
//
// These are intentionally aggressive. False positives (a slug that happens
// to look like a UUID) are preferable to leaked PII. The allowlist shape
// of the `TelemetryEvent` enum (structured props, not free text) already
// limits where free-form strings appear.

static URL_WITH_QUERY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://\S+\?\S+").expect("URL_WITH_QUERY regex"));

static BEARER_OR_AUTH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(bearer|basic|token|authorization:)\s*\S+").expect("BEARER_OR_AUTH regex")
});

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").expect("EMAIL regex")
});

static UUID_LIKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b[0-9a-fA-F]{8}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{12}\b",
    )
    .expect("UUID_LIKE regex")
});

static PROJECT_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(nyx_[A-Za-z0-9_\-]+|nyxid_[A-Za-z0-9_\-]+|sk-[A-Za-z0-9_\-]+|ghp_[A-Za-z0-9_\-]+|phc_[A-Za-z0-9_\-]+|ya29\.[A-Za-z0-9_\-\.]+)")
        .expect("PROJECT_TOKEN regex")
});

/// Redact a single string, applying all §6 egress rules in order.
/// Returns a new `String` if any redaction fired, otherwise the input
/// unchanged wrapped as `Cow::Borrowed`.
pub fn scrub_string(s: &str) -> std::borrow::Cow<'_, str> {
    let mut out = std::borrow::Cow::Borrowed(s);
    if URL_WITH_QUERY.is_match(&out) {
        out = std::borrow::Cow::Owned(
            URL_WITH_QUERY
                .replace_all(&out, "[URL_REDACTED]")
                .into_owned(),
        );
    }
    if BEARER_OR_AUTH.is_match(&out) {
        out = std::borrow::Cow::Owned(
            BEARER_OR_AUTH
                .replace_all(&out, "[AUTH_REDACTED]")
                .into_owned(),
        );
    }
    if EMAIL.is_match(&out) {
        out = std::borrow::Cow::Owned(EMAIL.replace_all(&out, "[EMAIL_REDACTED]").into_owned());
    }
    if UUID_LIKE.is_match(&out) {
        out = std::borrow::Cow::Owned(UUID_LIKE.replace_all(&out, "[UUID_REDACTED]").into_owned());
    }
    if PROJECT_TOKEN.is_match(&out) {
        out = std::borrow::Cow::Owned(
            PROJECT_TOKEN
                .replace_all(&out, "[TOKEN_REDACTED]")
                .into_owned(),
        );
    }
    out
}

/// Recursively scrub every string inside a JSON value, in place.
pub fn scrub_value(v: &mut Value) {
    match v {
        Value::String(s) => {
            if let std::borrow::Cow::Owned(scrubbed) = scrub_string(s) {
                *s = scrubbed;
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                scrub_value(item);
            }
        }
        Value::Object(obj) => {
            for (_k, val) in obj.iter_mut() {
                scrub_value(val);
            }
        }
        // Numbers, booleans, nulls: pass through.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_email() {
        let out = scrub_string("failed for user alice@example.com");
        assert!(out.contains("[EMAIL_REDACTED]"));
    }

    #[test]
    fn scrubs_bearer_token() {
        let out = scrub_string("Authorization: Bearer sk-foo-bar");
        assert!(out.contains("[AUTH_REDACTED]"));
    }

    #[test]
    fn scrubs_nyx_token_prefix() {
        let out = scrub_string("received nyx_nauth_1234abcd");
        assert!(out.contains("[TOKEN_REDACTED]"));
    }

    #[test]
    fn leaves_safe_string_alone() {
        let out = scrub_string("key.created");
        assert_eq!(out, "key.created");
    }

    #[test]
    fn scrubs_nested_json_strings() {
        let mut v = serde_json::json!({
            "route": "/users/alice@example.com",
            "inner": { "token": "nyx_nauth_secret" },
            "count": 42,
        });
        scrub_value(&mut v);
        let s = v.to_string();
        assert!(
            s.contains("[EMAIL_REDACTED]"),
            "expected email redaction in {s}"
        );
        assert!(
            s.contains("[TOKEN_REDACTED]"),
            "expected token redaction in {s}"
        );
        assert!(s.contains("42"), "expected numeric value preserved in {s}");
    }
}
