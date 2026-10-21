//! Default request headers for downstream proxying (NyxID#356).
//!
//! Admins attach headers to a `DownstreamService` and users attach headers
//! to their own `UserService`. Both collections feed the proxy injection
//! pipeline with the following precedence, lowest to highest:
//!
//! 1. Caller-supplied headers (already allowlisted in `proxy_service.rs`)
//! 2. `DownstreamService.default_request_headers` — admin catalog defaults
//! 3. `UserService.default_request_headers` — per-user overrides
//!
//! A header with `overridable: true` yields to whatever the layer below
//! already set (including caller-supplied values). The default is
//! `overridable: false`, so admin-set headers win unless explicitly
//! marked overridable.
//!
//! All default values use server-owned contextual ciphertext at rest. The API
//! accepts plaintext replacements or exact masked edits. The sensitive flag
//! controls management masking; explicitly public values remain visible after
//! authenticated decoding. Legacy plaintext reads exist only during migration.
//! Reserved authentication/hop-by-hop headers remain blocked by the denylist.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::errors::{AppError, AppResult};

/// Maximum number of default headers that can be configured per service.
pub const MAX_DEFAULT_HEADERS: usize = 16;

/// Maximum length of a header name (bytes).
pub const MAX_HEADER_NAME_LEN: usize = 256;

/// Maximum length of a header value (bytes).
pub const MAX_HEADER_VALUE_LEN: usize = 4096;

/// Header names (or name prefixes ending in `-*`) that must not be configured
/// as service defaults. Matches are case-insensitive.
///
/// Rationale:
/// - `authorization`, `cookie`, `set-cookie`: secrets — use `auth_method` / catalog credential flow
/// - `host`, `content-length`, `transfer-encoding`, `connection`, `upgrade`,
///   `te`, `trailer`, `proxy-*`: hop-by-hop / infrastructure headers that must
///   be recomputed per hop or would break reqwest's own framing
/// - `x-nyxid-*`, `x-forwarded-*`, `x-real-ip`: trust-boundary headers NyxID
///   sets itself; letting services override them would break identity
///   propagation and source-IP accounting
const DENYLISTED_HEADER_NAMES: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "trailer",
    "expect",
    "keep-alive",
    // `user-agent` has its own dedicated per-service knob (`custom_user_agent`
    // on both `DownstreamService` and `UserService`). Admins should use that
    // field, not default_request_headers, to avoid double-setting.
    "user-agent",
    // RFC 7239 replacement for the `x-forwarded-*` family. Default
    // headers are injected on every proxy path and would let an admin
    // spoof client IP / proto / host to downstreams that trust it — the
    // same trust-boundary concern as the `x-forwarded-` prefix below.
    "forwarded",
];

const DENYLISTED_HEADER_PREFIXES: &[&str] = &[
    "x-nyxid-",
    "x-forwarded-",
    "proxy-",
    // WebSocket handshake metadata is protocol-managed by tungstenite
    // (direct WS) and the node agent (node-routed WS) — `Sec-WebSocket-Key`
    // is a per-connection nonce, `Sec-WebSocket-Version` is the protocol
    // version, `Sec-WebSocket-Extensions` is negotiated, etc. Letting an
    // admin override any of these would break or weaken every WS proxy
    // for that service since defaults are merged into both transports.
    "sec-websocket-",
];

/// Additional exact-match denylist for well-known infra headers outside the
/// `x-forwarded-` prefix.
const DENYLISTED_EXTRA: &[&str] = &["x-real-ip"];

/// A single default request header attached to a service.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultRequestHeader {
    /// Case-insensitive HTTP header name. Stored as the admin typed it,
    /// but compared case-insensitively at proxy time.
    pub name: String,
    /// API plaintext replacement or the exact management mask.
    #[schema(value_type = String)]
    pub value: zeroize::Zeroizing<String>,
    /// When true, a value set by a lower-precedence layer (or supplied by
    /// the caller) wins and this default is skipped. Default false:
    /// admin-set values are authoritative.
    #[serde(default)]
    pub overridable: bool,
    /// All values use contextual encryption in storage. Sensitive values are masked
    /// in management responses. The ciphertext/version fields are storage-only.
    #[serde(default)]
    pub sensitive: bool,
}

/// In-process execution/authority representation; distinct from API DTOs and
/// persisted fields. Every owned decoded value clears on drop, including clones.
#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct ResolvedDefaultRequestHeader {
    pub name: String,
    pub value: zeroize::Zeroizing<String>,
    pub overridable: bool,
    pub sensitive: bool,
}
impl std::fmt::Debug for ResolvedDefaultRequestHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedDefaultRequestHeader")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("sensitive", &self.sensitive)
            .field("overridable", &self.overridable)
            .finish()
    }
}

/// Owned transport header values, including superseded entries, clear before
/// release. Protocol serialization remains the existing array of name/value pairs.
#[derive(Clone, Default, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct SecretHeaders(Vec<(String, String)>);
impl<'de> Deserialize<'de> for SecretHeaders {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Headers;
        impl<'de> serde::de::Visitor<'de> for Headers {
            type Value = SecretHeaders;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a sequence of header pairs")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut headers = SecretHeaders::default();
                while let Some((mut name, mut value)) =
                    seq.next_element::<(zeroize::Zeroizing<String>, zeroize::Zeroizing<String>)>()?
                {
                    headers
                        .0
                        .push((std::mem::take(&mut *name), std::mem::take(&mut *value)));
                }
                Ok(headers)
            }
        }
        deserializer.deserialize_seq(Headers)
    }
}
impl From<Vec<(String, String)>> for SecretHeaders {
    fn from(values: Vec<(String, String)>) -> Self {
        Self(values)
    }
}
impl std::ops::Deref for SecretHeaders {
    type Target = [(String, String)];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<'a> IntoIterator for &'a SecretHeaders {
    type Item = &'a (String, String);
    type IntoIter = std::slice::Iter<'a, (String, String)>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl SecretHeaders {
    pub fn push(&mut self, entry: (String, String)) {
        self.0.push(entry);
    }
    pub fn extend(&mut self, values: impl IntoIterator<Item = (String, String)>) {
        self.0.extend(values);
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&(String, String)) -> bool) {
        use zeroize::Zeroize;
        self.0.retain_mut(|entry| {
            if keep(entry) {
                true
            } else {
                entry.zeroize();
                false
            }
        });
    }
}
impl Drop for SecretHeaders {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}
impl std::fmt::Debug for SecretHeaders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretHeaders")
            .field("count", &self.0.len())
            .finish_non_exhaustive()
    }
}
impl PartialEq<Vec<(String, String)>> for SecretHeaders {
    fn eq(&self, other: &Vec<(String, String)>) -> bool {
        &self.0 == other
    }
}

pub trait HeaderEntry {
    fn name(&self) -> &str;
    fn value(&self) -> &str;
    fn overridable(&self) -> bool;
}
macro_rules! header_entry {
    ($header:ty) => {
        impl HeaderEntry for $header {
            fn name(&self) -> &str {
                &self.name
            }
            fn value(&self) -> &str {
                &self.value
            }
            fn overridable(&self) -> bool {
                self.overridable
            }
        }
    };
}
header_entry!(DefaultRequestHeader);
header_entry!(ResolvedDefaultRequestHeader);

/// Storage-only representation. Never accepted from API callers or serialized
/// into a management response. Missing format fields select the explicit legacy
/// plaintext contract during the configured migration window.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredDefaultRequestHeader {
    pub name: String,
    #[serde(default)]
    pub value: zeroize::Zeroizing<String>,
    #[serde(default)]
    pub overridable: bool,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub value_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub value_context_version: Option<u32>,
}

impl std::fmt::Debug for StoredDefaultRequestHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredDefaultRequestHeader")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("sensitive", &self.sensitive)
            .field("overridable", &self.overridable)
            .finish()
    }
}

/// Dedicated management conversion. Ciphertext is never decrypted for display.
/// Even malformed downgraded flags cannot make stored secret bytes public.
pub fn stored_list_for_response(
    list: Option<Vec<StoredDefaultRequestHeader>>,
) -> Option<Vec<DefaultRequestHeader>> {
    list.map(|list| {
        list.into_iter()
            .map(|header| {
                let sensitive = header.sensitive
                    || header.value_encrypted.is_some()
                    || header.value_context_version.is_some();
                DefaultRequestHeader {
                    name: header.name,
                    value: if sensitive {
                        REDACTED_PLACEHOLDER.to_owned().into()
                    } else {
                        header.value
                    },
                    sensitive,
                    overridable: header.overridable,
                }
            })
            .collect()
    })
}

#[cfg(test)]
impl From<DefaultRequestHeader> for StoredDefaultRequestHeader {
    fn from(header: DefaultRequestHeader) -> Self {
        Self {
            name: header.name,
            value: header.value,
            overridable: header.overridable,
            sensitive: header.sensitive,
            value_encrypted: None,
            value_context_version: None,
        }
    }
}

impl std::fmt::Debug for DefaultRequestHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultRequestHeader")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("overridable", &self.overridable)
            .field("sensitive", &self.sensitive)
            .finish()
    }
}

/// Returns true when `name` is denylisted as a default request header.
/// Case-insensitive.
pub fn is_denylisted_header_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return true;
    }
    if DENYLISTED_HEADER_NAMES.contains(&lower.as_str())
        || DENYLISTED_EXTRA.contains(&lower.as_str())
    {
        return true;
    }
    DENYLISTED_HEADER_PREFIXES
        .iter()
        .any(|p| lower.starts_with(p))
}

fn is_valid_header_name(name: &str) -> bool {
    // RFC 7230 token chars: VCHAR minus delimiters. reqwest's HeaderName uses
    // the same set. Keep it strict to avoid smuggling.
    !name.is_empty()
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn is_valid_header_value(value: &str) -> bool {
    // Matches `reqwest::header::HeaderValue::from_str` (which calls into
    // `http::HeaderValue::from_bytes`): the value may contain only
    // visible ASCII (0x21–0x7E), plus space (0x20) and horizontal tab
    // (0x09). Anything else — control bytes, DEL (0x7F), or any non-ASCII
    // byte — is rejected at send time, so we must reject it at store
    // time too. Otherwise a saved `café` silently breaks every future
    // proxy call for that service.
    value
        .bytes()
        .all(|b| b == b'\t' || (0x20..=0x7E).contains(&b))
}

/// Validate one normalized name/value before encryption or transport.
/// The storage service separately enforces list bounds, duplicates and masking.
pub(crate) fn validate_header_parts(name: &str, value: &str) -> AppResult<()> {
    if name.is_empty() {
        return Err(AppError::ValidationError(
            "default_request_headers: header name must not be empty".to_string(),
        ));
    }
    if name.len() > MAX_HEADER_NAME_LEN {
        return Err(AppError::ValidationError(format!(
            "default_request_headers: header name exceeds {MAX_HEADER_NAME_LEN} chars"
        )));
    }
    if !is_valid_header_name(name) {
        return Err(AppError::ValidationError(format!(
            "default_request_headers: invalid characters in header name '{}'",
            name
        )));
    }
    if is_denylisted_header_name(name) {
        return Err(AppError::ValidationError(format!(
            "default_request_headers: header '{}' is reserved and cannot be set as a default (use the service's auth_method for secrets, or remove it)",
            name
        )));
    }

    if value.len() > MAX_HEADER_VALUE_LEN {
        return Err(AppError::ValidationError(format!(
            "default_request_headers: value for '{}' exceeds {MAX_HEADER_VALUE_LEN} chars",
            name
        )));
    }
    if !is_valid_header_value(value) {
        return Err(AppError::ValidationError(format!(
            "default_request_headers: invalid characters in value for '{}' (CR / LF / NUL are not allowed)",
            name
        )));
    }

    Ok(())
}

/// Merge a list of caller-supplied / identity / delegated headers with default
/// headers from one or more layers.
///
/// Semantics (applied in order from lowest to highest precedence):
/// - `existing` is the starting header list (caller headers, identity props,
///   delegated provider headers, any custom user-agent). It may contain
///   multiple entries for the same name (HTTP allows it, and reqwest's
///   `HeaderMap::iter` emits one tuple per value).
/// - Each `layers` entry is applied in order. A non-overridable header
///   replaces **every** prior entry for that name — not just the first —
///   so a caller sending `x-scope: a` + `x-scope: b` cannot bypass a
///   non-overridable default by hiding a second copy behind the first.
///   An overridable header is only added when no prior entry exists
///   for that name.
///
/// Name matching is case-insensitive; the stored casing of the default entry
/// is preserved. The returned list is suitable for both the direct HTTP path
/// (attached with `RequestBuilder::header` which appends) and the node path
/// (`Vec<(String, String)>` serialized into the node-proxy frame).
pub fn merge_into_header_list<H: HeaderEntry>(
    existing: impl Into<SecretHeaders>,
    layers: &[&[H]],
) -> SecretHeaders {
    let mut existing = existing.into();
    for layer in layers {
        for h in *layer {
            let has_any = existing
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case(h.name()));
            if h.overridable() {
                if !has_any {
                    existing.push((h.name().to_owned(), h.value().to_owned()));
                }
                // else: caller / lower-layer value wins; leave untouched.
            } else {
                // Non-overridable: drop ALL existing case-insensitive
                // matches, then append exactly one default. Without the
                // retain, a duplicate caller header would survive and
                // reqwest / the node agent would emit both on the wire.
                existing.retain(|(n, _)| !n.eq_ignore_ascii_case(h.name()));
                existing.push((h.name().to_owned(), h.value().to_owned()));
            }
        }
    }
    existing
}

/// Placeholder returned in API responses in place of a sensitive value.
/// Kept as a constant so callers (tests, clients) can recognize it.
pub const REDACTED_PLACEHOLDER: &str = "•••••";

/// Produce an API-safe copy of a stored header list: entries with
/// `sensitive: true` have their `value` replaced with
/// [`REDACTED_PLACEHOLDER`]. The `name`, `overridable`, and `sensitive`
/// flags pass through unchanged so clients can still render the editor
/// UI correctly. Non-sensitive entries are returned as-is.
///
/// This is the canonical transformation applied at every outbound
/// boundary — `/services`, `/user-services`, `/keys`, `/catalog` — so
/// that the "Sensitive" UI affordance is honoured by the API. It does
/// NOT change proxy injection; the authenticated resolved value reaches the downstream.
pub fn redact_list_for_response(
    list: Option<Vec<DefaultRequestHeader>>,
) -> Option<Vec<DefaultRequestHeader>> {
    list.map(|entries| {
        entries
            .into_iter()
            .map(|h| DefaultRequestHeader {
                value: if h.sensitive {
                    if h.value.is_empty() {
                        String::new().into()
                    } else {
                        REDACTED_PLACEHOLDER.to_string().into()
                    }
                } else {
                    h.value
                },
                ..h
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    fn validate_headers(
        input: Vec<DefaultRequestHeader>,
    ) -> AppResult<Option<Vec<DefaultRequestHeader>>> {
        if input.is_empty() {
            return Ok(None);
        }
        if input.len() > MAX_DEFAULT_HEADERS {
            return Err(AppError::ValidationError(format!(
                "default_request_headers: too many entries (max {MAX_DEFAULT_HEADERS})"
            )));
        }

        let mut seen_lower: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(input.len());
        for h in input {
            let name = h.name.trim().to_string();
            let value = h.value.trim().to_string();

            validate_header_parts(&name, &value)?;

            let lower = name.to_ascii_lowercase();
            if !seen_lower.insert(lower) {
                return Err(AppError::ValidationError(format!(
                    "default_request_headers: duplicate header '{}' (names are compared case-insensitively)",
                    name
                )));
            }

            out.push(DefaultRequestHeader {
                name,
                value: value.into(),
                overridable: h.overridable,
                sensitive: h.sensitive,
            });
        }

        Ok(Some(out))
    }

    fn reconcile_with_stored(
        submitted: Vec<DefaultRequestHeader>,
        existing: Option<&[DefaultRequestHeader]>,
    ) -> Vec<DefaultRequestHeader> {
        submitted
            .into_iter()
            .map(|mut h| {
                if h.sensitive
                    && h.value.as_str() == REDACTED_PLACEHOLDER
                    && let Some(stored) = existing
                        .and_then(|rows| rows.iter().find(|r| r.name.eq_ignore_ascii_case(&h.name)))
                {
                    h.value = stored.value.clone();
                }
                h
            })
            .collect()
    }
    use super::*;

    fn h(name: &str, value: &str) -> DefaultRequestHeader {
        DefaultRequestHeader {
            name: name.to_string(),
            value: value.to_string().into(),
            overridable: false,
            sensitive: false,
        }
    }

    fn h_overridable(name: &str, value: &str) -> DefaultRequestHeader {
        DefaultRequestHeader {
            name: name.to_string(),
            value: value.to_string().into(),
            overridable: true,
            sensitive: false,
        }
    }

    #[test]
    fn empty_input_returns_none() {
        assert_eq!(validate_headers(vec![]).unwrap(), None);
    }

    #[test]
    fn validates_common_case() {
        let got = validate_headers(vec![h("x-openclaw-scopes", "operator.read,operator.write")])
            .unwrap()
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "x-openclaw-scopes");
        assert_eq!(got[0].value.as_str(), "operator.read,operator.write");
        assert!(!got[0].overridable);
    }

    #[test]
    fn trims_whitespace() {
        let got = validate_headers(vec![h("  X-Api-Version  ", "  v2  ")])
            .unwrap()
            .unwrap();
        assert_eq!(got[0].name, "X-Api-Version");
        assert_eq!(got[0].value.as_str(), "v2");
    }

    #[test]
    fn rejects_too_many() {
        let mut many = Vec::new();
        for i in 0..(MAX_DEFAULT_HEADERS + 1) {
            many.push(h(&format!("x-h-{i}"), "v"));
        }
        let err = validate_headers(many).unwrap_err();
        assert!(err.to_string().contains("too many"));
    }

    #[test]
    fn rejects_denylisted() {
        for name in [
            "Authorization",
            "authorization",
            "COOKIE",
            "Host",
            "User-Agent",
            // RFC 7239 Forwarded: blocked alongside the x-forwarded-*
            // prefix so admins can't spoof client IP / proto / host
            // via default headers.
            "Forwarded",
            "forwarded",
            "FORWARDED",
        ] {
            let err = validate_headers(vec![h(name, "v")]).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("reserved"),
                "expected reserved error for {name}, got {msg}"
            );
        }
    }

    #[test]
    fn rejects_denylisted_prefixes() {
        for name in [
            "x-nyxid-internal",
            "x-forwarded-for",
            "proxy-authorization",
            // sec-websocket-* is protocol-managed by tungstenite / the
            // node agent. Allowing admins to override would break the WS
            // handshake or downgrade the negotiated extensions.
            "sec-websocket-key",
            "Sec-WebSocket-Version",
            "sec-websocket-extensions",
            "Sec-WebSocket-Protocol",
            "SEC-WEBSOCKET-ACCEPT",
        ] {
            let err = validate_headers(vec![h(name, "v")]).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("reserved"),
                "expected reserved error for {name}, got {msg}"
            );
        }
    }

    #[test]
    fn rejects_invalid_name_chars() {
        for name in ["bad header", "bad:header", "bad/header", "bad(header)", ""] {
            let err = validate_headers(vec![h(name, "v")]).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("invalid") || msg.contains("empty"),
                "expected validation error for {name}, got {msg}"
            );
        }
    }

    #[test]
    fn rejects_crlf_in_value() {
        for value in ["a\r\nX-Inject: bad", "line\none", "with\0nul"] {
            let err = validate_headers(vec![h("x-test", value)]).unwrap_err();
            assert!(err.to_string().contains("invalid characters"));
        }
    }

    #[test]
    fn rejects_non_ascii_and_del_bytes() {
        // `reqwest::header::HeaderValue::from_str` rejects anything
        // outside visible ASCII + space + tab. Mirror that here so we
        // don't silently accept values the transport layer can't send.
        // `café`, Cyrillic, emoji, DEL (0x7F), and DC1 (0x11) should
        // all be rejected.
        let non_ascii_cases = ["café", "привет", "hi ☃", "bad\x7Fbad", "ctrl\x11byte"];
        for value in non_ascii_cases {
            let err = validate_headers(vec![h("x-test", value)]).unwrap_err();
            assert!(
                err.to_string().contains("invalid characters"),
                "expected rejection for {value:?}"
            );
        }
    }

    #[test]
    fn accepts_visible_ascii_with_tabs_and_spaces() {
        // Common header values should still pass.
        let ok = [
            "v2",
            "operator.read,operator.write",
            "bearer 123",
            "a=1; b=2",
            "col\tsep",
        ];
        for value in ok {
            let res = validate_headers(vec![h("x-test", value)]);
            assert!(
                res.is_ok(),
                "expected {value:?} to pass validation, got {res:?}"
            );
        }
    }

    #[test]
    fn rejects_duplicates_case_insensitive() {
        let err =
            validate_headers(vec![h("X-API-Version", "v1"), h("x-api-version", "v2")]).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn rejects_over_length() {
        let long_value = "v".repeat(MAX_HEADER_VALUE_LEN + 1);
        let err = validate_headers(vec![h("x-test", &long_value)]).unwrap_err();
        assert!(err.to_string().contains("exceeds"));

        let long_name = format!("x-{}", "a".repeat(MAX_HEADER_NAME_LEN));
        let err = validate_headers(vec![h(&long_name, "v")]).unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[test]
    fn merge_non_overridable_replaces_existing() {
        let existing = vec![("X-API-Version".to_string(), "v1".to_string())];
        let defaults = vec![h("X-API-Version", "v2")];
        let merged = merge_into_header_list(existing, &[&defaults]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].1, "v2");
    }

    #[test]
    fn merge_overridable_yields_to_existing() {
        let existing = vec![("X-API-Version".to_string(), "caller-value".to_string())];
        let defaults = vec![h_overridable("X-API-Version", "default-value")];
        let merged = merge_into_header_list(existing, &[&defaults]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].1, "caller-value");
    }

    #[test]
    fn merge_overridable_added_when_absent() {
        let existing: Vec<(String, String)> = vec![];
        let defaults = vec![h_overridable("X-API-Version", "default")];
        let merged = merge_into_header_list(existing, &[&defaults]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].1, "default");
    }

    #[test]
    fn merge_case_insensitive_match() {
        let existing = vec![("x-api-version".to_string(), "caller".to_string())];
        let defaults = vec![h_overridable("X-API-VERSION", "default")];
        let merged = merge_into_header_list(existing, &[&defaults]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].1, "caller");
    }

    #[test]
    fn merge_user_service_overrides_catalog() {
        let existing = vec![("x-caller".to_string(), "caller".to_string())];
        let catalog = vec![h("X-Scope", "catalog-scope")];
        let user = vec![h("X-Scope", "user-scope")];
        let merged = merge_into_header_list(existing, &[&catalog, &user]);
        let scope = merged
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("x-scope"));
        assert_eq!(scope.unwrap().1, "user-scope");
    }

    #[test]
    fn merge_catalog_persists_when_user_is_absent() {
        let existing: Vec<(String, String)> = vec![];
        let catalog = vec![h("X-Scope", "catalog-scope")];
        let user: Vec<DefaultRequestHeader> = vec![];
        let merged = merge_into_header_list(existing, &[&catalog, &user]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].1, "catalog-scope");
    }

    #[test]
    fn merge_non_overridable_strips_all_duplicate_caller_entries() {
        // A caller that sends the same header name twice must not be able
        // to slip a value past a non-overridable default. `HeaderMap::iter`
        // preserves duplicates, so the merge input can legitimately contain
        // both.
        let existing = vec![
            ("X-Scope".to_string(), "caller-a".to_string()),
            ("x-scope".to_string(), "caller-b".to_string()),
            ("X-Other".to_string(), "unrelated".to_string()),
        ];
        let defaults = vec![h("X-Scope", "forced")];
        let merged = merge_into_header_list(existing, &[&defaults]);

        let scope_entries: Vec<&(String, String)> = merged
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("x-scope"))
            .collect();
        assert_eq!(scope_entries.len(), 1, "duplicates must be collapsed");
        assert_eq!(scope_entries[0].1, "forced");
        // Unrelated header survives.
        assert!(
            merged
                .iter()
                .any(|(n, v)| n == "X-Other" && v == "unrelated")
        );
    }

    #[test]
    fn merge_overridable_keeps_all_duplicate_caller_entries() {
        // Overridable defaults must NOT touch caller values, including
        // duplicates, so the caller's intent is preserved verbatim.
        let existing = vec![
            ("X-Scope".to_string(), "caller-a".to_string()),
            ("x-scope".to_string(), "caller-b".to_string()),
        ];
        let defaults = vec![h_overridable("X-Scope", "default")];
        let merged = merge_into_header_list(existing, &[&defaults]);
        let scope_entries: Vec<&(String, String)> = merged
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("x-scope"))
            .collect();
        assert_eq!(scope_entries.len(), 2);
        assert_eq!(scope_entries[0].1, "caller-a");
        assert_eq!(scope_entries[1].1, "caller-b");
    }

    #[test]
    fn merge_user_overridable_yields_to_catalog_non_overridable() {
        let existing: Vec<(String, String)> = vec![];
        let catalog = vec![h("X-Scope", "catalog")]; // non-overridable
        let user = vec![h_overridable("X-Scope", "user")];
        let merged = merge_into_header_list(existing, &[&catalog, &user]);
        // Catalog's non-overridable ran first, setting X-Scope=catalog.
        // User's overridable sees X-Scope already present and yields.
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].1, "catalog");
    }

    #[test]
    fn redact_list_for_response_replaces_sensitive_values_only() {
        let input = Some(vec![
            DefaultRequestHeader {
                name: "X-Public".to_string(),
                value: "anyone-can-read".to_string().into(),
                overridable: false,
                sensitive: false,
            },
            DefaultRequestHeader {
                name: "X-Gateway-Token".to_string(),
                value: "super-secret-token".to_string().into(),
                overridable: false,
                sensitive: true,
            },
            DefaultRequestHeader {
                name: "X-Empty-Sensitive".to_string(),
                value: String::new().into(),
                overridable: true,
                sensitive: true,
            },
        ]);
        let redacted = redact_list_for_response(input).unwrap();
        assert_eq!(redacted[0].value.as_str(), "anyone-can-read");
        assert_eq!(redacted[1].value.as_str(), REDACTED_PLACEHOLDER);
        // name / overridable / sensitive flags preserved so the editor UI
        // can still render the row.
        assert_eq!(redacted[1].name, "X-Gateway-Token");
        assert!(redacted[1].sensitive);
        assert!(!redacted[1].overridable);
        // Empty sensitive stays empty (nothing to mask).
        assert_eq!(redacted[2].value.as_str(), "");
    }

    #[test]
    fn redact_list_for_response_passes_none_through() {
        assert!(redact_list_for_response(None).is_none());
    }

    fn sensitive_h(name: &str, value: &str) -> DefaultRequestHeader {
        DefaultRequestHeader {
            name: name.to_string(),
            value: value.to_string().into(),
            overridable: false,
            sensitive: true,
        }
    }

    #[test]
    fn reconcile_swaps_placeholder_for_stored_value() {
        // Round-trip: GET returned the placeholder, UI re-submits as-is
        // on another edit. The reconciler must restore the real value
        // before validation runs, otherwise the placeholder string would
        // land in the DB and every future proxy call would break when
        // the transport tried to serialize it.
        let existing = vec![sensitive_h("X-Gateway-Token", "real-secret-123")];
        let submitted = vec![sensitive_h("X-Gateway-Token", REDACTED_PLACEHOLDER)];
        let out = reconcile_with_stored(submitted, Some(&existing));
        assert_eq!(out[0].value.as_str(), "real-secret-123");
        assert!(out[0].sensitive);
    }

    #[test]
    fn reconcile_preserves_non_sensitive_and_new_values() {
        let existing = vec![sensitive_h("X-Gateway-Token", "secret")];
        let submitted = vec![
            sensitive_h("X-Gateway-Token", "new-secret"),
            h("X-Public", REDACTED_PLACEHOLDER),
        ];
        let out = reconcile_with_stored(submitted, Some(&existing));
        // Sensitive row with a new (non-placeholder) value: passes through.
        assert_eq!(out[0].value.as_str(), "new-secret");
        // Non-sensitive entries are never reconciled, even if their value
        // happens to equal the placeholder string.
        assert_eq!(out[1].value.as_str(), REDACTED_PLACEHOLDER);
        assert!(!out[1].sensitive);
    }

    #[test]
    fn reconcile_leaves_placeholder_for_unknown_name() {
        // New sensitive row with only the placeholder: no existing match
        // to resurrect, leave as-is so validation can flag it. (Right
        // now validation will accept the placeholder string; it is
        // still the user's problem to type a real value for a NEW
        // sensitive row — not the reconciler's to paper over.)
        let existing: Vec<DefaultRequestHeader> = Vec::new();
        let submitted = vec![sensitive_h("X-Brand-New", REDACTED_PLACEHOLDER)];
        let out = reconcile_with_stored(submitted, Some(&existing));
        assert_eq!(out[0].value.as_str(), REDACTED_PLACEHOLDER);
    }

    #[test]
    fn reconcile_handles_no_existing_list() {
        // No stored state: a placeholder for a new row is left alone so
        // the caller can decide how to treat it.
        let submitted = vec![sensitive_h("X-Gateway-Token", REDACTED_PLACEHOLDER)];
        let out = reconcile_with_stored(submitted, None);
        assert_eq!(out[0].value.as_str(), REDACTED_PLACEHOLDER);
    }

    #[test]
    fn reconcile_matches_case_insensitively() {
        let existing = vec![sensitive_h("x-gateway-token", "stored")];
        let submitted = vec![sensitive_h("X-GATEWAY-TOKEN", REDACTED_PLACEHOLDER)];
        let out = reconcile_with_stored(submitted, Some(&existing));
        assert_eq!(out[0].value.as_str(), "stored");
        // Casing from the submitted entry is preserved so renames
        // survive.
        assert_eq!(out[0].name, "X-GATEWAY-TOKEN");
    }
}

#[cfg(test)]
#[test]
fn default_header_debug_redacts_values_even_when_not_marked_sensitive() {
    let secret = "distinctive-default-header-d26a";
    for sensitive in [false, true] {
        let header = DefaultRequestHeader {
            name: "X-Test".into(),
            value: secret.to_owned().into(),
            overridable: false,
            sensitive,
        };
        assert!(!format!("{header:?}").contains(secret));
        assert_eq!(serde_json::to_value(header).unwrap()["value"], secret);
    }
}
